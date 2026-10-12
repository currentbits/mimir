import { useContext, useState } from 'react'
import { invoke } from '../api'
import { ago, tok, usd } from '../format'
import { Markdown } from '../markdown'
import { ask, Avatar, Empty, Icon } from '../ui'
import { when, useLoad, Bots, RUN_DOT } from '../shared'
import type { Run, Routine } from '../types'

type ToolCall = { id: number; tool: string; summary: string; decision: string; ok: boolean; result: string }

type Totals = { runs: number; errors: number; tokens_in: number; tokens_out: number; cost_usd: number }

export const RUN_TITLE: Record<Run['trigger'], string> = { message: 'Reply', routine: 'Routine', rehearsal: 'Rehearsal', tidy: 'Memory tidy-up', delegated: 'Task from another bot', import: 'Memory import', know_me: 'Getting to know you', check: 'Checked a suggestion' }

/** "Quiet check · Weekdays at 8:30 AM"; a deleted routine keeps its number. */
const runTitle = (r: Run, routine?: Routine) =>
  r.trigger !== 'routine' ? RUN_TITLE[r.trigger] : routine ? `${routine.quiet ? 'Quiet check' : routine.followup ? 'Follow-up' : routine.once ? 'One-off' : 'Routine'} · ${when(routine)}` : `Routine #${r.routine_id}`

export function ActivityView({ chatId }: { chatId: number }) {
  const runs = useLoad(() => invoke<Run[]>('runs', { chatId }), [chatId], 3000)
  const routines = useLoad(() => invoke<Routine[]>('routines', { chatId }), [chatId], 30000)
  const totals = useLoad(() => invoke<Totals>('run_totals', { chatId }), [chatId], 10000)
  const [open, setOpen] = useState<number | null>(null)
  const botOf = useContext(Bots)
  const list = runs.data ?? []
  const t = totals.data
  if (!list.length) return <Empty icon="activity" title="No activity yet">Each reply, routine and tidy-up shows up here with its model, tokens, cost and every tool it used.</Empty>
  return (
    <>
      {t && (
        <dl className="stats" aria-label="Last 30 days">
          <div className="stat"><dt>Runs · 30 days</dt><dd>{t.runs}{t.errors ? <small className="bad"> · {t.errors} failed</small> : null}</dd></div>
          <div className="stat"><dt>Tokens in / out</dt><dd>{tok(t.tokens_in)} <small>/ {tok(t.tokens_out)}</small></dd></div>
          <div className="stat"><dt>API-equivalent cost</dt><dd>{usd(t.cost_usd)}</dd><p>Subscriptions aren’t billed per token; this is what the same use would cost on the API.</p></div>
        </dl>
      )}
      <div className="group">
        {list.map(r => {
          const b = botOf(r.bot_id)
          return (
            <div key={r.id} className="run">
              <button className="runrow" onClick={() => setOpen(open === r.id ? null : r.id)} aria-expanded={open === r.id}>
                <span className={'dot ' + RUN_DOT[r.status]} />
                <span className="run-t" title={routines.data?.find(x => x.id === r.routine_id)?.prompt}><b>{(() => {
                  // A handed-over task names the bot that asked for it.
                  const asker = r.parent_run ? list.find(x => x.id === r.parent_run) : undefined
                  return asker ? `Task from ${botOf(asker.bot_id).name}` : runTitle(r, routines.data?.find(x => x.id === r.routine_id))
                })()}</b>
                  <small>{ago(r.started_at)}{r.status === 'error' ? ' · failed' : r.status === 'missed' ? ' · missed' : r.status === 'unchanged' ? ' · nothing changed, no AI run' : r.ended_at ? ` · ${r.ended_at - r.started_at}s` : ' · running'}{r.model ? ` · ${r.model}` : ''}{r.delivered ? ` · ${r.delivered.startsWith('not sent') ? 'not sent' : r.delivered}` : ''}</small></span>
                <span className="run-bot"><Avatar id={b.id} name={b.name} size={18} still /><span>{b.name}</span></span>
                <span className="num">{r.status === 'running' ? '…' : r.status === 'error' || r.status === 'missed' || r.status === 'unchanged' ? '—' : <span title={r.estimated ? 'Estimated from the text: this AI doesn’t say what it used' : r.cache_read ? `${tok(r.cache_read)} of the input came from the AI’s cache` : undefined}>{r.estimated ? '≈ ' : ''}{tok(r.tokens_in)} → {tok(r.tokens_out)}</span>}</span>
                <span className="num">{r.cost_usd != null ? usd(r.cost_usd) : '—'}</span>
                <span className="num">{r.tool_calls} {r.tool_calls === 1 ? 'tool' : 'tools'}</span>
                <Icon name="chevron" size={14} />
              </button>
              {open === r.id && <RunDetail run={r} />}
            </div>
          )
        })}
      </div>
      <ClearHistory chatId={chatId} onDone={() => runs.reload()} />
    </>
  )
}

/** Delete this chat's conversation and run log (memories and routines stay). */
function ClearHistory({ chatId, onDone }: { chatId: number; onDone: () => void }) {
  const [msg, setMsg] = useState<string | null>(null)
  const clear = async () => {
    if (!await ask('Delete this chat’s history: the conversation, the run log and what each action did, corrections, and the AI apps’ own copies? Memories and routines stay.', 'Delete history', true)) return
    invoke<number>('clear_history', { chatId }).then(n => { setMsg(`Deleted (${n} messages).`); onDone() }, e => setMsg(String(e)))
  }
  return (
    <p className="hint clear-history">
      <button type="button" className="btn btn-sm btn-ghost btn-danger" onClick={clear}><Icon name="trash" size={13} />Delete this chat’s history</button>
      {msg && <span>{msg}</span>}
    </p>
  )
}

function RunDetail({ run }: { run: Run }) {
  const calls = useLoad(() => invoke<ToolCall[]>('tool_calls', { runId: run.id }), [run.id], 3000)
  const items = calls.data ?? []
  const [stop, setStop] = useState<string | null>(null)
  return (
    <div className="run-detail">
      {run.status === 'running' && run.trigger === 'message' && (
        <div className="run-stop">
          <button className="btn btn-sm" disabled={stop === 'Stopping…'} onClick={() => { setStop('Stopping…'); invoke<unknown>('stop_run', { id: run.id }).then(r => setStop(r === false ? 'It had already finished.' : 'Stopped.'), e => setStop(String(e))) }}>
            <Icon name="x" size={13} />Stop this reply</button>
          {stop && stop !== 'Stopping…' && <span className="hint">{stop}</span>}
        </div>
      )}
      {run.status === 'running' && items.length === 0 && <p className="hint" style={{ margin: '4px 0' }}>Working on it…</p>}
      {run.error && <p className="error">{run.error}</p>}
      {run.status === 'ok' && run.reply != null && (
        run.reply.trim()
          ? <div className="run-reply"><div className="run-reply-label">Replied</div><div className="msg-body"><Markdown text={run.reply} /></div></div>
          : <p className="hint" style={{ margin: '4px 0' }}>Checked, and found nothing worth telling you.</p>
      )}
      {items.length === 0 && !run.error && run.status !== 'running' && <p className="hint" style={{ margin: '4px 0' }}>No tools used — it answered directly.</p>}
      {items.length > 0 && (
        <ol className="timeline">
          {items.map(c => (
            <li key={c.id} className={c.ok ? 'ok' : 'failed'}>
              <div className="tl-sum">{c.summary}{c.decision !== 'allow' && <span className={'badge ' + (c.decision === 'ask' ? 'pending' : 'danger')}>{c.decision === 'ask' ? 'Asked you' : 'Blocked'}</span>}</div>
              <div className="tl-res">{c.result}</div>
            </li>
          ))}
        </ol>
      )}
    </div>
  )
}
