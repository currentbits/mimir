import { useContext, useState } from 'react'
import { inDesktop, invoke, remoteServer } from '../api'
import { ahead, fullDay, fullTime, IDLE, schedule, tok, untilText, usd } from '../format'
import { ask, Avatar, Empty, Icon, Segmented, Switch } from '../ui'
import { Paused, pausedText } from './chat'
import { RUN_DOT, skillSlug, useLoad, Bots, when, RehearsalResult } from '../shared'
import type { Run, Delivery, Bot, Skill, Routine, Rehearsed } from '../types'

/** A routine's last 30 days and its last runs, newest first. */
type RoutineStats = { id: number; runs: number; tokens: number; cost_usd: number | null; failed: number; missed: number; unchanged?: number; ignored?: string[]; last: { status: Run['status']; at: number; tokens: number }[] }

const RUN_WORD: Record<Run['status'], string> = { ok: 'Ran', error: 'Failed', missed: 'Missed', running: 'Running', unchanged: 'Nothing changed, so no AI run' }

/** Under a routine: its last runs as dots (oldest left), and what it used in 30 days. */
function RoutineHealth({ s, onReset }: { s?: RoutineStats; onReset?: () => void }) {
  if (!s || (!s.last.length && !s.ignored?.length)) return null
  const each = s.runs ? Math.round(s.tokens / s.runs) : 0
  return (
    <div className="rh">
      <span className="rh-dots" aria-label="Last runs">{[...s.last].reverse().map((m, i) => <span key={i} className={'dot ' + RUN_DOT[m.status]} title={`${RUN_WORD[m.status]} · ${fullTime(m.at)}${m.tokens ? ` · ${tok(m.tokens)} tokens` : ''}`} />)}</span>
      {s.runs > 0 && <span>30 days: {s.runs} {s.runs === 1 ? 'run' : 'runs'}{each ? ` · about ${tok(each)} tokens each` : ''}{s.cost_usd ? ` · ${usd(s.cost_usd)} API-equivalent` : ''}</span>}
      {!!s.unchanged && <span title="Mimir looked and nothing had changed, so the AI didn't run">{s.unchanged} unchanged, free</span>}
      {!!s.ignored?.length && <span title={`Left out because they change on every load:\n${s.ignored.join('\n')}`}>
        ignoring {s.ignored.length} {s.ignored.length === 1 ? 'line' : 'lines'} that change on every load{onReset && <> · <button type="button" className="linkish" onClick={onReset}>Reset</button></>}</span>}
      {(s.failed > 0 || s.missed > 0) && <span className="bad">{[s.failed && `${s.failed} failed`, s.missed && `${s.missed} missed`].filter(Boolean).join(' · ')}</span>}
    </div>
  )
}

/** One-click quiet checks: read-only, and silent unless something needs attention. */
const CHECKINS = [
  { title: 'Morning task check', cron: '30 8 * * Mon-Fri', icon: 'check', prompt: 'Look at my open tasks in memory. If any are due today or overdue, remind me which and why. Otherwise say nothing.' },
  { title: 'Evening look-ahead', cron: '0 19 * * Sun-Thu', icon: 'moon', prompt: 'Check my memory for anything happening tomorrow (appointments, deadlines, people to contact). If there is something, give me a short heads-up.' },
  { title: 'Watch a web page', cron: '0 */3 * * *', icon: 'globe', prompt: 'Tell me if the page reports a problem, or what changed if it matters.', watch: 'https://example.com/status' },
]

// ---- schedule picker: plain choices in, cron out (anything else stays editable as "Custom") ----
type WhenMode = 'daily' | 'weekdays' | 'weekly' | 'hours' | 'custom' | 'mail' | 'webhook'

type When = { mode: WhenMode; time: string; day: string; hours: number; cron: string; rule: string }

const WEEK = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

function toWhen(cron: string): When {
  const base: When = { mode: 'custom', time: '08:00', day: 'Mon', hours: 3, cron, rule: '' }
  if (cron.trim() === 'webhook') return { ...base, mode: 'webhook' }
  if (cron.trim().startsWith('mail:')) return { ...base, mode: 'mail', rule: cron.trim().slice(5).trim() }
  const f = cron.trim().split(/\s+/)
  if (f.length !== 5) return base
  const [m, h, dom, mon, dow] = f
  const at = /^\d+$/.test(m) && /^\d+$/.test(h) ? `${h.padStart(2, '0')}:${m.padStart(2, '0')}` : null
  if (at && dom === '*' && mon === '*') {
    if (dow === '*') return { ...base, mode: 'daily', time: at }
    if (/^(mon-fri|1-5)$/i.test(dow)) return { ...base, mode: 'weekdays', time: at }
    const day = WEEK.find(d => d.toLowerCase() === dow.toLowerCase())
    if (day) return { ...base, mode: 'weekly', time: at, day }
  }
  const n = h.match(/^\*\/(\d+)$/)?.[1]
  if (m === '0' && n && dom === '*' && mon === '*' && dow === '*') return { ...base, mode: 'hours', hours: +n }
  return base
}

function toCron(w: When): string {
  const [h, m] = w.time.split(':').map(Number)
  return w.mode === 'daily' ? `${m} ${h} * * *` : w.mode === 'weekdays' ? `${m} ${h} * * Mon-Fri` : w.mode === 'weekly' ? `${m} ${h} * * ${w.day}`
    : w.mode === 'hours' ? `0 */${w.hours} * * *` : w.mode === 'mail' ? (w.rule.trim() ? `mail:${w.rule.trim()}` : '') : w.mode === 'webhook' ? 'webhook' : w.cron
}

type RoutineDraft = { prompt: string; botId: number; quiet: boolean; when: When; delivery: Delivery; watch: string; urgent: string; firstOnly?: boolean; until: string }

/** A date input's day as the routine's end (through that day), and back; '' and 0 are none. */
const endOf = (day: string) => day ? Math.floor(new Date(`${day}T23:59:59`).getTime() / 1000) : 0
const dayOf = (ts?: number) => { if (!ts) return ''; const d = new Date(ts * 1000); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}` }

function RoutineForm({ draft, setDraft, bots, onCancel, onSave, inline, skills = [] }: {
  draft: RoutineDraft; setDraft: (d: RoutineDraft) => void; bots: Bot[]; onCancel: () => void; onSave: () => void; inline?: boolean; skills?: Skill[]
}) {
  const runnable = skills.filter(k => k.status === 'active' && (k.bot_id === draft.botId || k.bot_id === 0))
  const w = draft.when
  const cron = toCron(w)
  const when = (patch: Partial<When>) => setDraft({ ...draft, when: { ...w, ...patch } })
  const readable = schedule(cron)
  return (
    <form className={(inline ? 'routine-inline' : 'editor-card') + ' editor routine-form'} onSubmit={e => { e.preventDefault(); onSave() }}>
      <label className="field-label"><span>What should it do?</span>
        <textarea className="textarea" autoFocus value={draft.prompt} placeholder="e.g. Send me the top 3 tech headlines, one line each, with links."
          onChange={e => setDraft({ ...draft, prompt: e.target.value })} /></label>
      {runnable.length > 0 && (
        <label className="field-label"><span>Or run one of its skills</span>
          <select className="select" value={runnable.find(k => draft.prompt.trim() === `/${skillSlug(k.name)}`)?.id ?? ''}
            onChange={e => { const k = runnable.find(x => x.id === Number(e.target.value)); if (k) setDraft({ ...draft, prompt: `/${skillSlug(k.name)}` }) }}>
            <option value="">Choose a skill…</option>{runnable.map(k => <option key={k.id} value={k.id}>{k.name}</option>)}
          </select></label>
      )}
      <div className="field-label"><span>When</span>
        <Segmented size="sm" label="How often" value={w.mode} onChange={mode => when(mode === 'custom' ? { mode, cron } : { mode })}
          options={[{ id: 'daily', label: 'Every day' }, { id: 'weekdays', label: 'Weekdays' }, { id: 'weekly', label: 'Weekly' }, { id: 'hours', label: 'Every few hours' }, { id: 'mail', label: 'When email arrives' }, { id: 'webhook', label: 'When called' }, { id: 'custom', label: 'Custom' }]} />
        <div className="er when-row">
          {w.mode === 'weekly' && (
            <select className="select" aria-label="Day" value={w.day} onChange={e => when({ day: e.target.value })}>
              {WEEK.map(d => <option key={d} value={d}>{fullDay(d)}</option>)}
            </select>
          )}
          {(w.mode === 'daily' || w.mode === 'weekdays' || w.mode === 'weekly') && <input className="input" type="time" aria-label="Time" value={w.time} required onChange={e => when({ time: e.target.value })} />}
          {w.mode === 'hours' && (
            <select className="select" aria-label="Every how many hours" value={w.hours} onChange={e => when({ hours: Number(e.target.value) })}>
              {[1, 2, 3, 4, 6, 8, 12].map(n => <option key={n} value={n}>Every {n === 1 ? 'hour' : `${n} hours`}</option>)}
            </select>
          )}
          {w.mode === 'custom' && <input className="input mono" aria-label="Cron schedule" value={w.cron} placeholder="30 8 * * Mon-Fri" onChange={e => when({ cron: e.target.value })} />}
          {w.mode === 'mail' && <input className="input mono" aria-label="Which email" value={w.rule} placeholder='from:acme.com subject:invoice' onChange={e => when({ rule: e.target.value })} />}
        </div>
        {w.mode === 'mail' && <span className="hint"><code>from:</code> an address or domain, <code>subject:</code> a word or "a phrase", and any other words the email must contain. Mimir checks new mail every 2 minutes; mail that doesn’t match costs nothing.</span>}
        {w.mode === 'mail' && draft.firstOnly !== undefined && (
          <div className="quiet-row">
            <Switch label="Only the first one" checked={draft.firstOnly} onChange={firstOnly => setDraft({ ...draft, firstOnly })} />
            <div><b>Only the first one</b><span>Runs for the first matching email, then it’s gone (“tell me when it arrives”).</span></div>
          </div>
        )}
        {w.mode === 'webhook' && <span className="hint">Runs each time its address on your Mimir server is called, with what was sent. After saving, the routine shows the address and its secret.</span>}
        {w.mode === 'custom' && <span className="hint">{readable !== cron ? `Runs: ${readable}` : 'Cron format: minute hour day-of-month month weekday, e.g. 30 8 * * Mon-Fri; or calendar:60:outside for an hour before each meeting with someone outside'}</span>}
      </div>
      <label className="field-label"><span>Until (optional)</span>
        <input className="input" type="date" style={{ width: 180 }} value={draft.until} min={dayOf(Date.now() / 1000)} onChange={e => setDraft({ ...draft, until: e.target.value })} />
        <span className="hint">Its last run is the last one on or before that day; then it’s gone.</span>
      </label>
      <div className="quiet-row">
        <Switch label="Quiet" checked={draft.quiet} onChange={quiet => setDraft({ ...draft, quiet })} />
        <div><b>Quiet</b><span>Read-only, and it only messages you when something needs your attention.</span></div>
      </div>
      {draft.quiet && (
        <label className="field-label"><span>Watch (optional)</span>
          <input className="input" value={draft.watch} placeholder="https://… or mail" onChange={e => setDraft({ ...draft, watch: e.target.value })} />
          <span className="hint">A page, a feed, or <code>mail</code> for new unread email. Mimir looks first and only runs the AI when it changed, so checking often costs almost nothing.</span>
        </label>
      )}
      <div className="field-label"><span>Send it</span>
        <Segmented size="sm" label="Send it" value={draft.delivery} onChange={delivery => setDraft({ ...draft, delivery })}
          options={[{ id: 'now', label: 'Right away' }, { id: 'digest', label: 'In my daily digest' }, { id: 'card', label: 'Update a card' }, { id: 'log', label: 'Just log it' }]} />
        <span className="hint">{draft.delivery === 'now' ? 'A message (and a notification) as soon as it runs.' : draft.delivery === 'digest' ? 'Collected into one message a day, at the time set in Settings.' : draft.delivery === 'card' ? 'Nothing is sent: it rewrites a card pinned at the top of the chat (a list, bills due). Tick items there yourself.' : 'Nothing is sent; see what it found in Activity.'}</span>
      </div>
      <label className="field-label"><span>Come through right away if (optional)</span>
        <input className="input" value={draft.urgent} maxLength={200} placeholder="e.g. my flight is delayed or cancelled" onChange={e => setDraft({ ...draft, urgent: e.target.value })} />
        <span className="hint">When it would otherwise wait (in the digest, log only, or quiet hours). The message says it came through because of this. At most 3 a day, all bots together.</span>
      </label>
      <div className="er" style={{ alignItems: 'center' }}>
        {bots.length > 1 && !inline && (
          <select className="select" aria-label="Bot" style={{ width: 180 }} value={draft.botId} onChange={e => setDraft({ ...draft, botId: Number(e.target.value) })}>
            {bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
          </select>
        )}
        <span className="grow hint">{readable !== cron ? readable : ''}</span>
        <button type="button" className="btn btn-ghost" onClick={onCancel}>Cancel</button>
        <button className="btn btn-primary" disabled={!draft.prompt.trim() || !cron.trim()}>{inline ? 'Save' : 'Add routine'}</button>
      </div>
    </form>
  )
}

/** A webhook routine's address and secret, shown when asked for (the secret is made then). */
function WebhookAddress({ chatId, id }: { chatId: number; id: number }) {
  const [hook, setHook] = useState<{ path: string; secret: string } | string | null>(null)
  if (!hook) return <button type="button" className="linkish" onClick={() => invoke<{ path: string; secret: string }>('webhook', { chatId, id }).then(setHook, e => setHook(String(e)))}>Show its address and secret</button>
  if (typeof hook === 'string') return <p className="error">{hook}</p>
  const base = (inDesktop ? remoteServer()?.url ?? '' : location.origin).replace(/\/$/, '')
  return (
    <div className="hint selectable">
      <div>POST <code>{base}{hook.path}</code></div>
      <div>Secret: <code>{hook.secret}</code>, sent as <code>Authorization: Bearer …</code>, or as an HMAC-SHA256 of the body in <code>X-Hub-Signature-256: sha256=…</code> (GitHub’s webhook secret works as is). What’s sent is treated as outside content.</div>
    </div>
  )
}

export function RoutinesView({ chatId, bots }: { chatId: number; bots: Bot[] }) {
  const rows = useLoad(() => invoke<Routine[]>('routines', { chatId }), [chatId])
  const stats = useLoad(() => invoke<RoutineStats[]>('routine_stats', { chatId }), [chatId], 30000)
  const botOf = useContext(Bots)
  const pausedOf = useContext(Paused)
  const [err, setErr] = useState<string | null>(null)
  const [draft, setDraft] = useState<RoutineDraft | null>(null)
  const [editing, setEditing] = useState<{ id: number; draft: RoutineDraft } | null>(null)
  const [runningId, setRunningId] = useState<number | null>(null)
  const [rehearsal, setRehearsal] = useState<{ id: number; r: Rehearsed | null } | null>(null)
  const rehearse = (id: number) => { setRehearsal({ id, r: null }); invoke<Rehearsed>('rehearse', { chatId, routineId: id }).then(r => setRehearsal({ id, r }), e => { setRehearsal(null); setErr(String(e)) }) }
  const skillRows = useLoad(() => invoke<Skill[]>('skills', { chatId }), [chatId])
  const list = rows.data ?? []
  const deliver = (id: number, delivery: Delivery) => invoke('set_delivery', { chatId, id, delivery })
  const watch = (id: number, w: string) => invoke('set_watch', { chatId, id, watch: w.trim() })
  const urgent = (id: number, rule: string) => invoke('set_urgent', { chatId, id, rule })
  const ends = (id: number, day: string) => invoke('set_ends', { chatId, id, ends: endOf(day) })
  const update = (r: Routine, patch: { cron?: string; prompt?: string; quiet?: boolean; paused?: boolean; delivery?: Delivery; watch?: string; urgent?: string; until?: string }) =>
    // A new end goes with the schedule, so the two are checked together and a refused change keeps both.
    invoke('update_routine', { chatId, id: r.id, cron: r.cron, prompt: r.prompt, quiet: r.quiet, paused: r.paused, ...patch, ends: patch.until !== undefined && patch.until !== dayOf(r.ends) ? endOf(patch.until) : null })
      .then(() => patch.watch !== undefined && patch.watch.trim() !== (r.watch ?? '') ? watch(r.id, patch.watch) : null)
      .then(() => patch.delivery && patch.delivery !== r.delivery ? deliver(r.id, patch.delivery) : null)
      .then(() => patch.urgent !== undefined && patch.urgent.trim() !== (r.urgent ?? '') ? urgent(r.id, patch.urgent) : null)
      .then(() => { setErr(null); setEditing(null); rows.reload() }, e => setErr(String(e)))
  const start = (cron: string, prompt: string, quiet: boolean, w = '') => { setErr(null); setDraft({ prompt, quiet, botId: bots[0]?.id ?? 1, when: toWhen(cron), delivery: 'now', watch: w, urgent: '', firstOnly: false, until: '' }) }
  const add = () => draft && invoke<Routine>('add_routine', { chatId, botId: draft.botId, cron: toCron(draft.when), prompt: draft.prompt, quiet: draft.quiet, firstOnly: draft.firstOnly })
    .then(async r => { if (draft.quiet && draft.watch.trim()) await watch(r.id, draft.watch); return r })
    .then(async r => { if (draft.urgent.trim()) await urgent(r.id, draft.urgent); return r })
    // ponytail: a bad end leaves the routine made without one; the error says why, and Edit sets it
    .then(async r => { if (draft.until) await ends(r.id, draft.until); return r })
    .then(r => draft.delivery !== 'now' ? deliver(r.id, draft.delivery) : null)
    .then(() => { setDraft(null); setErr(null); rows.reload() }, e => setErr(String(e)))
  return (
    <>
      {err && <p className="error">{err}</p>}
      {list.length === 0 ? (
        <Empty icon="clock" title="Nothing scheduled">
          Ask a bot for something recurring — “every weekday at 7:30, send me the top 3 tech headlines” — or to keep an eye on something and only speak up when it matters.
        </Empty>
      ) : (
        <>
          <div className="section-title" style={{ marginTop: 0 }}><h2>Scheduled</h2><span className="count">{list.length}</span></div>
          <div className="group">
            {list.map(r => {
              const b = botOf(r.bot_id)
              const held = pausedOf(r.bot_id)
              if (editing?.id === r.id) return (
                <RoutineForm key={r.id} inline skills={skillRows.data ?? []} draft={editing.draft} setDraft={d => setEditing({ id: r.id, draft: d })} bots={bots} onCancel={() => setEditing(null)}
                  onSave={() => update(r, { cron: toCron(editing.draft.when), prompt: editing.draft.prompt, quiet: editing.draft.quiet, delivery: editing.draft.delivery, watch: editing.draft.quiet ? editing.draft.watch : '', urgent: editing.draft.urgent, until: editing.draft.until })} />
              )
              return (
                <div className={'routine' + (r.paused || held !== undefined || r.goal_held ? ' paused' : '')} key={r.id}>
                  <span className={'ri' + (r.quiet ? ' quiet' : '')}><Icon name={r.quiet ? 'moon' : 'clock'} /></span>
                  <div>
                    <div className="rt">{when(r)}{r.followup && <span className="badge" title="The bot booked this on its own, to check back. Delete it to cancel.">Follow-up</span>}{r.for_goal && <span className="badge" title="Booked for this chat's goal: it goes when the goal is done or removed">For the goal</span>}{!!r.ends && <span className="badge" title="Its last run is the last one before this; then it’s gone">Until {untilText(r.ends)}</span>}{r.quiet && <span className="badge pending" title="Read-only; speaks up only when something needs your attention">Quiet</span>}{(r.paused || held !== undefined) ? <span className="badge">Paused</span> : r.goal_held && <span className="badge" title="Its goal is paused or done; it runs again when the goal is resumed">Paused (goal)</span>}{r.delivery === 'digest' && <span className="badge" title="Collected into your daily digest">In digest</span>}{r.delivery === 'log' && <span className="badge" title="Not sent; see Activity">Log only</span>}{r.delivery === 'card' && <span className="badge" title="Updates a card pinned at the top of the chat; nothing is sent">On a card</span>}{r.urgent && <span className="badge" title={`Comes through right away if: ${r.urgent}`}>Urgent if…</span>}{r.quiet && r.watch && <span className="badge" title="Mimir looks first and only runs the AI when this changed">Watching {r.watch === 'mail' ? 'mail' : (() => { try { return new URL(r.watch).host } catch { return r.watch } })()}</span>}</div>
                    <div className="rp selectable">{r.prompt}</div>
                    <div className="rm"><Avatar id={b.id} name={b.name} size={16} still /><span>{b.name}</span><span>·</span><span>{r.paused ? 'Won’t run until you turn it back on' : held !== undefined ? `${pausedText(held)} with ${b.name}; ${r.once ? 'it runs once then' : 'runs in between are skipped, not caught up'}` : r.goal_held ? 'Waits while its goal is paused' : r.next_fire >= IDLE ? `${r.cron === 'webhook' ? 'Waits for a call' : r.cron.startsWith('mail:') ? 'Waits for a matching email' : 'No matching event on the calendar this week'}${r.once ? ', then it’s gone' : ''}` : `${r.once ? 'Runs' : 'Next run'} ${ahead(r.next_fire)}${r.once ? ', then it’s gone' : ''}`}</span></div>
                    {r.cron === 'webhook' && <WebhookAddress chatId={chatId} id={r.id} />}
                    <RoutineHealth s={stats.data?.find(x => x.id === r.id)} onReset={() => invoke('reset_watch_noise', { chatId, id: r.id }).then(stats.reload, e => setErr(String(e)))} />
                    {rehearsal?.id === r.id && (rehearsal.r ? <RehearsalResult r={rehearsal.r} onClose={() => setRehearsal(null)} /> : <p className="hint">Rehearsing… it runs for real, but anything it would send or change is only described.</p>)}
                  </div>
                  <div className="routine-actions">
                    <Switch label={r.paused ? `Resume: ${when(r)}` : `Pause: ${when(r)}`} checked={!r.paused} onChange={on => update(r, { paused: !on })} />
                    <button className="btn btn-ghost btn-sm" disabled={runningId === r.id} title="Run it once now; the result goes to the chat"
                      onClick={() => { setRunningId(r.id); invoke<string>('run_routine', { chatId, id: r.id }).then(() => setErr(null), e => setErr(String(e))).finally(() => { setRunningId(null); rows.reload(); stats.reload() }) }}>
                      {runningId === r.id ? 'Running…' : 'Run now'}
                    </button>
                    <button className="btn btn-ghost btn-sm" disabled={rehearsal?.id === r.id && !rehearsal.r} title="Run it once without sending or changing anything: see what it would do"
                      onClick={() => rehearse(r.id)}>Rehearse</button>
                    {/* A one-off has no schedule to edit: change its words by asking the bot, or delete it. */}
                    {!r.once && <button className="btn btn-ghost btn-sm" onClick={() => setEditing({ id: r.id, draft: { prompt: r.prompt, quiet: r.quiet, botId: r.bot_id, when: toWhen(r.cron), delivery: r.delivery ?? 'now', watch: r.watch ?? '', urgent: r.urgent ?? '', until: dayOf(r.ends) } })}>Edit</button>}
                    <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Delete" aria-label="Delete routine" onClick={() => ask('Delete this routine?', 'Delete', true).then(ok => { if (ok) void invoke('delete_routine', { chatId, id: r.id }).then(rows.reload, e => setErr(String(e))) })}><Icon name="trash" size={14} /></button>
                  </div>
                </div>
              )
            })}
          </div>
        </>
      )}

      <div className="section-title"><h2>Add a routine</h2></div>
      <p className="hint" style={{ margin: '-4px 0 12px' }}>Anything a bot should do on a schedule. Quiet ones only message you when something needs your attention.</p>
      {draft ? <RoutineForm skills={skillRows.data ?? []} draft={draft} setDraft={setDraft} bots={bots} onCancel={() => setDraft(null)} onSave={add} /> : (
        <div className="presets four">
          <button className="preset new" onClick={() => start('0 8 * * *', '', false)}>
            <Icon name="plus" size={18} /><b>New routine</b><span>Pick what and when</span>
          </button>
          {CHECKINS.map(c => (
            <button key={c.title} className="preset" onClick={() => start(c.cron, c.prompt, true, c.watch)}>
              <Icon name={c.icon} size={18} /><b>{c.title}</b><span>{schedule(c.cron)}</span>
            </button>
          ))}
        </div>
      )}
    </>
  )
}
