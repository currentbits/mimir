import { copyText, inDesktop, insecureServer, invoke, saveDownload, keystore, localInvoke, platform, probe, remoteServer, setWebToken, connectServer, shortcut, trayName, webToken } from './api'
import { createContext, Fragment, useCallback, useMemo, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { ask, askText, AskHost, tell, Avatar, Empty, Group, Icon, Row, Segmented, Switch, type Presence } from './ui'
import { ago, ahead, dayLabel, fullDay, fullTime, pigment, PIGMENTS, schedule, setLooks, tok, usd, type Look } from './format'
import { Markdown, openLink } from './markdown'
import { moodOf, type Mood } from './face'
import { EXTRAS, EYES, SHAPES, traits } from './face-core'

type ChannelStatus = { kind: 'telegram' | 'slack' | 'discord'; connected: boolean; handle: string | null; error: string | null }
type Status = { bot_id: number; provider: string; pair_code: string | null; channels: ChannelStatus[]; error: string | null }
type Bot = {
  id: number; name: string; persona: string; provider: string; model: string; base_url: string; tools: string
  has_telegram_token: boolean; has_slack: boolean; has_discord: boolean
  /** Its creature, as you chose it (JSON of Look); empty = the one its id gives it. */
  look?: string
}

const parseLook = (s?: string): Look => { try { return s ? JSON.parse(s) : {} } catch { return {} } }
const any = <T,>(list: readonly T[]) => list[Math.floor(Math.random() * list.length)]
/** A fresh creature: what a new bot starts as (change it, or shuffle again). */
const randomLook = (): Look => ({ shape: any(SHAPES), eyes: any(EYES), extra: any(EXTRAS), color: any(PIGMENTS) })

const LOOK_NAMES: Record<string, string> = {
  pebble: 'Pebble', dome: 'Dome', bean: 'Bean', tall: 'Tall', oval: 'Oval', round: 'Round', wide: 'Wide apart', bead: 'Beady',
  cheeks: 'Rosy cheeks', sprout: 'Sprout', '': 'Nothing', antenna: 'Antenna', freckles: 'Freckles',
}

/** Build a bot's creature: colour, shape, eyes and one detail, each shown on the creature itself. */
function LookPicker({ id, look, onChange }: { id: number; look: Look; onChange: (l: Look) => void }) {
  const t = traits(id, look)
  const now: Look = { shape: t.shape, eyes: t.eyes, extra: t.extra, color: pigment(id, look) }
  const set = (p: Look) => onChange({ ...now, ...p })
  const Opts = ({ part, list }: { part: 'shape' | 'eyes' | 'extra'; list: readonly string[] }) => (
    <div className="look-opts" role="radiogroup" aria-label={part === 'extra' ? 'Detail' : part === 'eyes' ? 'Eyes' : 'Shape'}>
      {list.map(v => (
        <button key={v} type="button" role="radio" aria-checked={now[part] === v} className={'look-opt' + (now[part] === v ? ' on' : '')} title={LOOK_NAMES[v]} onClick={() => set({ [part]: v })}>
          <Avatar id={id} name="" size={34} still look={{ ...now, [part]: v }} /><small>{LOOK_NAMES[v]}</small>
        </button>
      ))}
    </div>
  )
  return (
    <div className="look-picker">
      <div className="look-row"><span>Colour</span>
        <div className="swatches" role="radiogroup" aria-label="Colour">{PIGMENTS.map(c => (
          <button key={c} type="button" role="radio" aria-checked={now.color === c} aria-label={`Colour ${c}`} className={'swatch' + (now.color === c ? ' on' : '')} style={{ background: c }} onClick={() => set({ color: c })} />
        ))}</div></div>
      <div className="look-row"><span>Shape</span><Opts part="shape" list={SHAPES} /></div>
      <div className="look-row"><span>Eyes</span><Opts part="eyes" list={EYES} /></div>
      <div className="look-row"><span>Detail</span><Opts part="extra" list={EXTRAS} /></div>
      <div className="look-row"><span /><button type="button" className="btn btn-sm btn-ghost" onClick={() => onChange(randomLook())}><Icon name="retry" size={13} />Surprise me</button></div>
    </div>
  )
}
type Secrets = { telegram_token: string | null; slack_bot_token: string | null; slack_app_token: string | null; discord_token: string | null }
type Action = { tool: string; summary: string; decision: string; ok: boolean; result?: string }
type Message = { id: number; bot_id: number; role: string; content: string; ts: number; actions?: Action[]; run_id?: number }

/** What a reply did, in a few words: ["memory", "Saved to memory · Dentist"]. Lookups and listings aren't worth a line. */
function actionLine(a: Action): [string, string] | null {
  const s = a.summary
  const q = (re: RegExp) => s.match(re)?.[1] ?? ''
  const line: [string, string] | null =
    a.tool === 'memory_store' ? ['memory', `${a.result?.startsWith('Not saved') ? 'Suggested for memory' : 'Saved to memory'} · ${q(/^Remember "(.*?)":/)}`]
    : a.tool === 'memory_forget' ? ['trash', `Forgot · ${q(/"(.*)"$/)}`]
    : a.tool === 'routine_create' ? ['clock', s.startsWith('Once at') ? `Reminder · ${q(/^Once at (.*?):/)}` : `Scheduled · ${schedule(q(/^Create routine \[(.*?)\]/))}`]
    : a.tool === 'routine_delete' ? ['clock', 'Deleted a routine']
    : a.tool === 'ask_bot' ? ['user', `Asked ${q(/^Ask (.*?):/)}`]
    : a.tool === 'web_fetch' ? ['globe', `Read ${q(/^Read https?:\/\/([^/\s]+)/) || 'a page'}`]
    : a.tool === 'skill_save' ? ['bolt', `Skill to save · ${q(/^Save as a skill: (.*)$/)}`]
    : a.tool.includes('__') ? [a.tool.startsWith('browser__') ? 'browser' : 'plug', s.split(' (')[0]]
    : null
  if (!line) return null
  if (a.decision === 'deny') return ['x', `Not allowed · ${line[1]}`]
  if (!a.ok) return ['alert', `Didn’t work · ${line[1]}`]
  return line
}

function Actions({ actions, chatId, runId, onChange }: { actions: Action[]; chatId: number; runId?: number; onChange: () => void }) {
  const [open, setOpen] = useState<number | null>(null)
  const [handoff, setHandoff] = useState(false)
  const lines = actions.map(a => [a, actionLine(a)] as const).filter((l): l is readonly [Action, [string, string]] => !!l[1])
  if (!lines.length) return null
  const shown = lines.slice(0, 4)
  // A routine the bot just scheduled: the chip opens the routine as saved, to check, run or undo it.
  const routineOf = (a: Action) => a.tool === 'routine_create' && a.ok ? Number(a.result?.match(/#(\d+)/)?.[1]) || null : null
  // A skill the bot proposed because you asked: the chip opens it, to Save or not.
  const skillOf = (a: Action) => a.tool === 'skill_save' && a.ok ? Number(a.result?.match(/skill #(\d+)/)?.[1]) || null : null
  const [skillOpen, setSkillOpen] = useState<number | null>(null)
  // A memory the bot saved: Undo right on the chip (the same as Undo in Memory → Learned this week).
  const memOf = (a: Action) => a.tool === 'memory_store' && a.ok ? Number(a.result?.match(/^stored #(\d+)/)?.[1]) || null : null
  const [undone, setUndone] = useState<number[]>([])
  const undo = (id: number) => invoke<boolean>('undo_memory', { chatId, id }).then(ok => { if (ok) setUndone(u => [...u, id]); onChange() }, e => tell(`Couldn't undo that: ${e}`))
  return (
    <>
      <div className="msg-actions">
        {shown.map(([a, [icon, text]], i) => {
          const rid = routineOf(a)
          const cls = 'msg-action' + (icon === 'x' || icon === 'alert' ? ' off' : '')
          // A task handed to another bot: the chip opens what that bot did and said.
          if (a.tool === 'ask_bot' && a.ok && runId) return (
            <button key={i} type="button" className={cls + ' link' + (handoff ? ' on' : '')} onClick={() => setHandoff(h => !h)} aria-expanded={handoff}><Icon name={icon} size={12} />{text}<Icon name="chevron" size={11} className="chev" /></button>
          )
          const mid = memOf(a)
          if (mid) return undone.includes(mid)
            ? <span key={i} className={cls + ' off'}><Icon name="memory" size={12} />{text.replace('Saved to memory', 'Undone')}</span>
            : <span key={i} className={cls}><Icon name={icon} size={12} />{text}
              <button type="button" className="chip-undo" onClick={() => undo(mid)} aria-label={`Undo: ${text}`}>Undo</button></span>
          const sid = skillOf(a)
          if (sid) return (
            <button key={i} type="button" className={cls + ' link' + (skillOpen === sid ? ' on' : '')} onClick={() => setSkillOpen(skillOpen === sid ? null : sid)} aria-expanded={skillOpen === sid}><Icon name={icon} size={12} />{text}<Icon name="chevron" size={11} className="chev" /></button>
          )
          return rid
            ? <button key={i} type="button" className={cls + ' link' + (open === rid ? ' on' : '')} onClick={() => setOpen(open === rid ? null : rid)} aria-expanded={open === rid}><Icon name={icon} size={12} />{text}<Icon name="chevron" size={11} className="chev" /></button>
            : <span key={i} className={cls}><Icon name={icon} size={12} />{text}</span>
        })}
        {lines.length > shown.length && <span className="msg-action more">+{lines.length - shown.length} more in Activity</span>}
      </div>
      {open && <RoutineCard id={open} chatId={chatId} onChange={onChange} />}
      {skillOpen && <SkillCard id={skillOpen} chatId={chatId} />}
      {handoff && runId && <HandoffCard runId={runId} />}
    </>
  )
}

/** What you wrote. With a file, the bot got its text; here it's your words and the file's name. */
function Said({ text }: { text: string }) {
  // A reply to one of the bot's messages (Telegram's swipe-to-reply): the quote above what they wrote.
  const re = text.match(/^<replying_to_your_message>([\s\S]*?)<\/replying_to_your_message>\n/)
  if (re) return <><span className="msg-quote">{re[1]}</span><Said text={text.slice(re[0].length)} /></>
  const doc = text.match(/^([\s\S]*?)\n\n\[Attached: (.+?)\. Its text follows/)
  if (!doc) return <>{text}</>
  return <>{doc[1] !== '(I sent you a file.)' && <>{doc[1]}<br /></>}<span className="msg-file"><Icon name="file" size={13} />{doc[2]}</span></>
}

type Step = { id: number; tool: string; summary: string; decision: string; ok: boolean; result: string; ts: number }

/** What another bot did for this reply: its steps and its answer, with what it took. */
function HandoffCard({ runId }: { runId: number }) {
  const data = useLoad(() => invoke<{ run: Run; steps: Step[] }[]>('run_children', { runId }), [runId], 15000)
  const botOf = useContext(Bots)
  if (!data.data) return null
  if (!data.data.length) return <div className="routine-card gone">No record of that hand-off (it may be from before Mimir kept them).</div>
  return <>{data.data.map(({ run, steps }) => {
    const b = botOf(run.bot_id)
    return (
      <div key={run.id} className="routine-card handoff">
        <div className="rc-head"><Avatar id={b.id} name={b.name} size={26} {...(run.status === 'error' ? { mood: 'sad' as const, still: true } : run.ended_at ? { still: true } : { mood: 'thinking' as const })} />
          <div><b>{b.name}</b> <small className="hint">{run.status === 'error' ? 'didn’t finish' : run.ended_at ? `took ${run.ended_at - run.started_at}s` : 'working…'}{run.model ? ` · ${run.model}` : ''}</small></div></div>
        {steps.length > 0 && <ol className="run-steps">{steps.map(s => <li key={s.id} className={s.ok ? 'ok' : 'failed'}><Icon name={s.ok ? 'check' : 'x'} size={11} />{s.summary}</li>)}</ol>}
        {run.reply ? <div className="msg-body selectable"><Markdown text={run.reply.length > 1500 ? run.reply.slice(0, 1500) + '…' : run.reply} /></div>
          : run.error ? <p className="error" style={{ margin: 0 }}>{run.error}</p> : null}
      </div>
    )
  })}</>
}

/** A routine, as saved, right where it was made: what and when, and run it now, pause or delete it. */
/** A command name for a skill, as the bots read it: "Book padel" → "book-padel". */
const skillSlug = (name: string) => name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '')

/** A skill a bot proposed in the chat because you asked it to save one: Save it, or not. */
function SkillCard({ id, chatId }: { id: number; chatId: number }) {
  const rows = useLoad(() => invoke<Skill[]>('skills', { chatId }), [chatId], 10000)
  const [err, setErr] = useState<string | null>(null)
  if (!rows.data) return null
  const k = rows.data.find(x => x.id === id)
  if (!k) return <div className="routine-card gone">Not saved.</div>
  const decide = (accept: boolean) => invoke('decide_skill', { chatId, id, accept }).then(rows.reload, e => setErr(String(e)))
  return (
    <div className="routine-card skill-card">
      <div className="rc-head">
        <span className="ri"><Icon name="bolt" size={15} /></span>
        <div className="rc-main">
          <div className="rt">{k.name}{k.status === 'active' && <span className="badge live">Saved</span>}</div>
          <div className="rm">{k.description}</div>
          <div className="skill-steps selectable"><Markdown text={k.body} /></div>
          {k.reason && <div className="rm">{k.reason}</div>}
        </div>
      </div>
      <div className="rc-actions">
        {k.status === 'proposed'
          ? <><button className="btn btn-sm" onClick={() => decide(true)}>Save</button><button className="btn btn-sm btn-ghost" onClick={() => decide(false)}>Not now</button></>
          : <span className="hint">Use it any time with <code>/{skillSlug(k.name)}</code>, in this chat or your messaging app.</span>}
        {err && <span className="error">{err}</span>}
      </div>
    </div>
  )
}

type Rehearsed = { reply: string; would: string[]; looked: string[] }

/** What a rehearsal found, and what it would have done (nothing was sent or changed). */
function RehearsalResult({ r, onClose }: { r: Rehearsed; onClose: () => void }) {
  return (
    <div className="rehearsal">
      <div className="rehearsal-head"><b>Rehearsal</b><span className="hint grow">Nothing was sent or changed.</span><button className="btn btn-ghost btn-sm" onClick={onClose}>Close</button></div>
      {r.would.length > 0
        ? <div><span className="field-label">It would have</span><ul>{r.would.map((w, i) => <li key={i} className="selectable">{w}</li>)}</ul></div>
        : <span className="hint">It wouldn’t change or send anything.</span>}
      {r.looked.length > 0 && <span className="hint">It looked at: {r.looked.join(' · ')}</span>}
      <div><span className="field-label">What it would say</span><div className="selectable"><Markdown text={r.reply.trim() || 'Nothing needed your attention.'} /></div></div>
    </div>
  )
}

function RoutineCard({ id, chatId, onChange }: { id: number; chatId: number; onChange: () => void }) {
  const rows = useLoad(() => invoke<Routine[]>('routines', { chatId }), [chatId], 10000)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const r = rows.data?.find(x => x.id === id)
  if (!rows.data) return null
  if (!r) return <div className="routine-card gone">This routine has been deleted.</div>
  const runNow = () => {
    setBusy(true); setNote(null)
    invoke<string>('run_routine', { chatId, id }).then(t => { setNote(t === 'Nothing needed your attention.' ? t : 'Ran just now. The result is below.'); onChange() }, e => setNote(String(e))).finally(() => setBusy(false))
  }
  const update = (patch: Partial<Routine>) => invoke('update_routine', { chatId, id, cron: r.cron, prompt: r.prompt, quiet: r.quiet, paused: r.paused, ...patch }).then(rows.reload, e => setNote(String(e)))
  return (
    <div className="routine-card">
      <div className="rc-head">
        <span className={'ri' + (r.quiet ? ' quiet' : '')}><Icon name={r.quiet ? 'moon' : 'clock'} size={15} /></span>
        <div className="rc-main">
          <div className="rt">{when(r)}{r.quiet && <span className="badge pending">Quiet</span>}{r.paused && <span className="badge">Paused</span>}{r.delivery === 'digest' && <span className="badge">In digest</span>}{r.delivery === 'log' && <span className="badge">Log only</span>}</div>
          <div className="rp selectable">{r.prompt}</div>
          <div className="rm">{r.paused ? 'Paused' : `Next run ${ahead(r.next_fire)}`} · change it any time in Routines</div>
        </div>
      </div>
      <div className="rc-actions">
        <button className="btn btn-sm" disabled={busy} onClick={runNow}><Icon name="bolt" size={13} />{busy ? 'Running…' : 'Run now'}</button>
        <button className="btn btn-sm btn-ghost" onClick={() => update({ paused: !r.paused })}>{r.paused ? 'Resume' : 'Pause'}</button>
        <button className="btn btn-sm btn-ghost btn-danger" onClick={() => ask('Delete this routine?', 'Delete', true).then(ok => { if (ok) void invoke('delete_routine', { chatId, id }).then(rows.reload, e => tell(`Couldn't delete it: ${e}`)) })}>Delete</button>
        {busy && <span className="typing" aria-hidden="true"><i /><i /><i /></span>}
        {note && <span className="hint">{note}</span>}
      </div>
    </div>
  )
}
type Chat = { chat_id: number; title: string; memories: number; channel: string; guest?: boolean }
type Kind = 'convention' | 'knowledge' | 'task'
type EntryRow = {
  id: number; kind: Kind; title: string; body: string; source: string; created_at: number
  status: 'active' | 'superseded' | 'forgotten'; superseded_by: number | null; access_count: number; pinned?: boolean; background?: boolean
  origin?: '' | 'you' | 'bot' | 'tidy' | 'import' | 'know_me'; bot_id?: number | null; tainted?: boolean; missteps?: number; valid_until?: number | null; asked?: string | null
}
type Delivery = 'now' | 'digest' | 'log'
type Routine = { id: number; bot_id: number; quiet: boolean; paused: boolean; chat_id: number; cron: string; prompt: string; next_fire: number; delivery: Delivery; once?: boolean; watch?: string }
/** When a routine runs, in words: its schedule, or the one time a one-off (a reminder) runs. */
const when = (r: Routine) => r.once ? `Once · ${fullTime(r.next_fire)}` : schedule(r.cron)
type Tab = 'chat' | 'memory' | 'skills' | 'routines' | 'activity'
type View = Tab | 'settings' | 'bot' | 'inbox' | 'computer' | 'meetings'
type ComputerStatus = { available: boolean; enabled: boolean; running: boolean }
type Run = {
  id: number; bot_id: number; chat_id: number; reply?: string | null; parent_run?: number | null; trigger: 'message' | 'routine' | 'rehearsal' | 'tidy' | 'delegated' | 'import' | 'know_me' | 'check'; routine_id: number | null; provider: string; model: string | null
  status: 'running' | 'ok' | 'error' | 'missed' | 'unchanged'; error: string | null; started_at: number; ended_at: number | null
  tokens_in: number; tokens_out: number; cost_usd: number | null; tool_calls: number
  delivered?: string | null; repeats?: number; cache_read?: number; estimated?: boolean
}
type ToolCall = { id: number; tool: string; summary: string; decision: string; ok: boolean; result: string }
type Totals = { runs: number; errors: number; tokens_in: number; tokens_out: number; cost_usd: number }
/** A routine's last 30 days and its last runs, newest first. */
type RoutineStats = { id: number; runs: number; tokens: number; cost_usd: number | null; failed: number; missed: number; unchanged?: number; ignored?: string[]; last: { status: Run['status']; at: number; tokens: number }[] }
const RUN_DOT: Record<Run['status'], string> = { ok: 'live', error: 'err', missed: 'missed', running: 'wait', unchanged: 'same' }
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
type Approval = { id: number; bot_id: number; chat_id: number; tool: string; summary: string; created_at: number; expires_at: number; kind?: 'rule' | 'guarded' | 'login' | 'plan' | 'bot' | 'send' | 'takeover'; site?: string | null; parked?: boolean; allowed_before?: number }

/** What an approval would do, in a few words, and whether it can become an "always allow". */
function approvalRisk(a: Approval): { icon: string; label: string; tone: string; always: boolean } {
  if (a.kind === 'takeover') return { icon: 'laptop', label: 'A step only you should do. Take over on the live screen, then press Give back; the bot carries on.', tone: 'login', always: false }
  if (a.kind === 'login') return { icon: 'key', label: 'Types a saved login. Every use asks you.', tone: 'login', always: false }
  if (a.kind === 'send') return { icon: 'send', label: 'Sends an email as you, to the address shown first. Every email asks you.', tone: 'rule', always: false }
  if (a.kind === 'bot') return { icon: 'user', label: 'Makes a new bot with this role, on the same AI account. Every new bot asks you.', tone: 'rule', always: false }
  if (a.kind === 'plan') return { icon: 'check', label: 'One OK for the whole plan: these steps won\'t ask again during this reply.', tone: 'rule', always: false }
  if (a.kind === 'guarded') return { icon: 'shield', label: 'Asking because this conversation read outside content (a web page, a file, a tool’s result).', tone: 'guarded', always: false }
  const t = a.tool, server = t.includes('__') ? t.split('__')[0] : null
  const label =
    t === 'web_fetch' || t === 'browser__browser_navigate' ? `Opens ${a.site ?? 'a website'}`
    : /forget|delete|remove|drop/.test(t) ? 'Deletes something'
    : t === 'ask_bot' ? 'Hands a task to another bot'
    : server && server !== 'browser' ? `Uses ${server}, which can change things`
    : 'Changes your data'
  const icon = label.startsWith('Opens') ? 'globe' : label.startsWith('Deletes') ? 'trash' : label.startsWith('Hands') ? 'user' : server ? 'plug' : 'pencil'
  return { icon, label, tone: label.startsWith('Deletes') ? 'delete' : 'rule', always: true }
}
type Suggestion = { id: number; action: 'add' | 'update' | 'forget' | 'demote'; entry_id: number | null; type: Kind; title: string; body: string; reason: string; check?: string | null; tainted?: boolean }
type VaultEntry = { name: string; domains: string[] }
type Rule = { tool: string; description: string; action: 'allow' | 'ask' | 'deny'; default: string }

const LOCAL = 0
/** The incognito chat (built-in chat only): messages stay in the window; nothing is stored. */
const INCOGNITO = -1
/** The built-in chat: on this computer, or the web chat of a self-hosted server. */
const LOCAL_NAME = inDesktop && !remoteServer() ? 'This computer' : 'Web chat'
/** Where tokens and keys are kept. */
const SECRETS_AT = inDesktop && !remoteServer() ? keystore : "your server's private database"
const CHANNEL: Record<string, { name: string; icon: string }> = {
  telegram: { name: 'Telegram', icon: 'telegram' }, slack: { name: 'Slack', icon: 'slack' },
  discord: { name: 'Discord', icon: 'discord' }, local: { name: 'Built-in chat', icon: inDesktop && !remoteServer() ? 'laptop' : 'globe' },
}
const online = (s?: Status) => s?.channels.filter(c => c.connected) ?? []
/** What's wrong with a bot that isn't working, in words (its account, or a messaging app that won't connect). */
const troubleOf = (s?: Status): string | null =>
  s?.error ?? (s?.channels.filter(c => c.error && !c.connected)
    .map(c => (c.error!.startsWith(CHANNEL[c.kind].name) ? c.error! : `${CHANNEL[c.kind].name}: ${c.error}`)).join(' · ') || null)
type Busy = { bot_id: number; chat_id: number; trigger: string; step: string | null }
/** A bot at work, as its face: the step a reply is on, else what kind of work it is. */
const busyMood = (w: Busy): Mood => w.step ? moodOf(w.step, '') : w.trigger === 'tidy' || w.trigger === 'import' || w.trigger === 'know_me' ? 'remembering' : w.trigger === 'teach' ? 'writing' : 'thinking'
const BUSY_KIND: Record<string, string> = { routine: 'Running a routine', delegated: 'On a task for another bot', tidy: 'Tidying memory', check: 'Checking a preference', import: 'Importing memories', know_me: 'Getting to know you', teach: 'Writing a skill' }
const busyLabel = (w: Busy) => w.step ? `${w.step}…` : BUSY_KIND[w.trigger] ?? 'Replying…'
const presenceOf = (s?: Status): Presence => !s ? 'off' : online(s).length ? 'live' : s.error || s.channels.some(c => c.error) ? 'err' : s.channels.length ? 'wait' : 'off'

/** Bot id → bot, provided by Main. */
const Bots = createContext<(id: number) => { id: number; name: string }>(id => ({ id, name: `Bot ${id}` }))

/** Poll a loader every `ms` and on demand. */
function useLoad<T>(load: () => Promise<T>, deps: unknown[], ms = 5000) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  // Each change of what's loaded (another chat, another bot) starts a new generation: an answer for the one before,
  // still on its way, is dropped instead of landing over the new one.
  const gen = useRef(0)
  const mounted = useRef(false)
  const reload = useCallback(() => {
    const g = gen.current
    return load().then(d => { if (g === gen.current) { setData(d); setError(null) } }, e => { if (g === gen.current) setError(String(e)) })
  }, deps) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    // What was shown for the one before goes too (a banner, a list), rather than standing in until the new one loads.
    if (mounted.current) { gen.current++; setData(null); setError(null) }
    mounted.current = true
    reload()
    if (ms <= 0) return // load once, no polling
    const t = setInterval(reload, ms)
    return () => clearInterval(t)
  }, [reload, ms])
  return { data, error, reload }
}

/** Browser on a self-hosted server: ask for the admin token first. */
export default function Root() {
  const [authed, setAuthed] = useState(inDesktop || !!webToken())
  useEffect(() => {
    const out = () => { if (!inDesktop) { setWebToken(null); setAuthed(false) } }
    window.addEventListener('mimir-unauthorized', out)
    return () => window.removeEventListener('mimir-unauthorized', out)
  }, [])
  return authed ? <App /> : <SignIn onDone={() => setAuthed(true)} />
}

function SignIn({ onDone }: { onDone: () => void }) {
  const [token, setToken] = useState('')
  const [err, setErr] = useState<string | null>(null)
  return (
    <div className="center-screen">
      <form className="signin" onSubmit={e => {
        e.preventDefault()
        probe('', token.trim()).then(() => { setWebToken(token.trim()); onDone() }, e => setErr(String(e)))
      }}>
        <img className="mark" src="/logo.png" alt="" />
        <h1>Sign in to your bots</h1>
        <p>Run <code>mimir admin-token</code> on your server to see the token.</p>
        <input className="input" type="password" aria-label="Admin token" placeholder="Admin token" value={token} onChange={e => setToken(e.target.value)} autoFocus autoComplete="current-password" />
        <button className="btn btn-primary btn-lg" style={{ justifyContent: 'center' }} disabled={!token.trim()}>Sign in</button>
        {err && <p className="error">{err}</p>}
      </form>
    </div>
  )
}

function App() {
  const [onboarded, setOnboarded] = useState<boolean | null>(null)
  useEffect(() => { invoke<{ onboarded: boolean }>('info').then(i => setOnboarded(i.onboarded), () => setOnboarded(true)) }, [])
  if (onboarded === null) return null
  return <><AskHost />{onboarded ? <Main /> : <Onboarding onDone={() => setOnboarded(true)} />}</>
}

// ---------------- onboarding ----------------

type Account = { installed: boolean; signed_in?: boolean; version?: string; plan?: string | null; account?: string | null; server?: string; base?: string; models?: Model[] }
type Model = { id: string; tools: boolean | null; name?: string }

const ACCOUNTS: Record<string, { name: string; mark: string; desc: string; install: string; signin: string }> = {
  claude: { name: 'Claude', mark: 'C', desc: 'Your Claude subscription, through Claude Code', install: 'Install Claude Code from claude.com/claude-code, then check again.', signin: 'Open Terminal, run claude, and sign in with your Claude account.' },
  chatgpt: { name: 'ChatGPT', mark: 'G', desc: 'Your ChatGPT Plus or Pro plan. Sign in; nothing to install', install: '', signin: 'Sign in with your ChatGPT account. You set how much of your plan Mimir may use in ChatGPT’s settings.' },
  codex: { name: 'Codex CLI', mark: 'G', desc: 'Your ChatGPT subscription, through the Codex CLI', install: 'Install the Codex CLI with npm install -g @openai/codex, then check again.', signin: 'Open Terminal and run codex login to sign in with ChatGPT.' },
  grok: { name: 'Grok', mark: 'X', desc: 'Your SuperGrok or X Premium+ subscription, through Grok Build', install: 'Install Grok Build with curl -fsSL https://x.ai/cli/install.sh | bash in Terminal, then check again.', signin: 'Sign in to Grok for Mimir. It keeps its own login, separate from your grok setup.' },
  openrouter: { name: 'OpenRouter', mark: 'R', desc: 'One key for Claude, GPT, Gemini, Llama and hundreds more', install: '', signin: '' },
  xai: { name: 'xAI API', mark: 'X', desc: 'Grok models billed per token, with an xAI API key', install: '', signin: '' },
  local: { name: 'Local model', mark: 'L', desc: 'Ollama or LM Studio on this computer. Private and free.', install: "Install Ollama from ollama.com and run ollama pull qwen3:4b in Terminal (or start LM Studio's local server), then check again.", signin: 'Ollama is running but has no models yet. Run ollama pull qwen3:4b in Terminal, then check again.' },
  openai: { name: 'API key', mark: 'K', desc: 'OpenAI, or any OpenAI-compatible service', install: '', signin: '' },
}
const ACCOUNT_ORDER = ['claude', 'chatgpt', 'codex', 'grok', 'openrouter', 'xai', 'local', 'openai']
/** Accounts that run on an API key, with what the key looks like. */
const KEYED: Record<string, string> = { openrouter: 'OpenRouter key (sk-or-…)', xai: 'xAI key (xai-…)', openai: 'API key (sk-…)' }
const MODEL_HINT: Record<string, string> = {
  claude: 'Default — or sonnet, opus, haiku', chatgpt: 'Default — or one your plan offers', codex: 'Default — or gpt-5.5', grok: 'Default — or grok-4.7-build-fast', openrouter: 'Auto — or pick one, e.g. anthropic/claude-sonnet-5.5',
  xai: 'grok-4.7', local: "Pick a model you've downloaded", openai: 'gpt-5-mini',
}

/** One-click OpenRouter sign-in (desktop app): you approve in the browser and the key comes back to the app. */
function ConnectOpenRouter({ onDone, children = 'Connect OpenRouter' }: { onDone: () => void; children?: React.ReactNode }) {
  const [waiting, setWaiting] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const run = useRef(0)
  if (!inDesktop || remoteServer()) return null
  const go = async () => {
    const me = ++run.current
    setWaiting(true); setErr(null)
    try {
      const { url } = await invoke<{ url: string }>('openrouter_connect')
      await localInvoke('open_url', { url })
      await invoke('openrouter_connect_wait')
      if (me === run.current) { setWaiting(false); onDone() }
    } catch (e) { if (me === run.current) { setWaiting(false); setErr(String(e)) } }
  }
  if (waiting) return (
    <span className="connect-wait" role="status"><span className="typing" aria-hidden="true"><i /><i /><i /></span>Approve Mimir in your browser…
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => { run.current++; setWaiting(false); invoke('openrouter_cancel').catch(() => {}) }}>Cancel</button></span>
  )
  return <>
    <button type="button" className="btn" onClick={go}><Icon name="bolt" size={14} />{children}</button>
    {err && <span className="error" style={{ margin: 0 }}>{err}</span>}
  </>
}

/** Sign in with ChatGPT: bots then run on your ChatGPT plan, no CLI. In the desktop app the browser comes back to the app;
 *  a server can't receive that (it lands on your own computer), so you paste the address your browser ended up on.
 *  `place` puts the button (or the waiting note) where it belongs, and the paste form below it. */
function SignInChatGPT({ onDone, place }: { onDone: () => void; place: (control: React.ReactNode, below: React.ReactNode) => React.ReactNode }) {
  const [flow, setFlow] = useState<{ url: string; paste: boolean } | null>(null)
  const [pasted, setPasted] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const run = useRef(0)
  const done = () => { run.current++; setFlow(null); setPasted(''); onDone() }
  const go = async () => {
    const me = ++run.current
    setBusy(true); setErr(null)
    try {
      const f = await invoke<{ url: string; paste: boolean }>('chatgpt_sign_in')
      setFlow(f); setBusy(false)
      if (inDesktop) localInvoke('open_url', { url: f.url }).catch(() => {})
      if (!f.paste) { await invoke('chatgpt_sign_in_wait'); if (me === run.current) done() }
    } catch (e) { if (me === run.current) { setBusy(false); setFlow(null); setErr(String(e)) } }
  }
  const finish = () => { setErr(null); invoke('chatgpt_sign_in_paste', { url: pasted.trim() }).then(done, e => setErr(String(e))) }
  const cancel = () => { run.current++; setFlow(null); setPasted(''); invoke('chatgpt_sign_in_cancel').catch(() => {}) }
  const error = err && <span className="error" style={{ margin: 0 }}>{err}</span>
  if (flow && !flow.paste) return place(
    <span className="connect-wait" role="status"><span className="typing" aria-hidden="true"><i /><i /><i /></span>
      Sign in in your browser… <a href={flow.url} onClick={openLink}>Open it again</a>
      <button type="button" className="btn btn-sm btn-ghost" onClick={cancel}>Cancel</button></span>, null
  )
  if (flow) return place(<span className="hint">Waiting for the address…</span>,
    <form className="row-block" onSubmit={e => { e.preventDefault(); if (pasted.trim()) finish() }}>
      <span className="hint">1. <a href={flow.url} target="_blank" rel="noreferrer" onClick={openLink}>Sign in with ChatGPT</a>{inDesktop ? ' (it opened in your browser)' : ''}.
        2. Your browser then lands on a page that doesn’t load, at an address starting with <code>http://127.0.0.1</code>. Copy that whole address and paste it here.</span>
      <div className="editor" style={{ margin: 0 }}><div className="er">
        <input className="input" placeholder="http://127.0.0.1:1455/auth/callback?code=…" value={pasted} onChange={e => setPasted(e.target.value)} autoComplete="off" aria-label="The address your browser ended up on" autoFocus />
        <button className="btn" disabled={!pasted.trim()}>Finish</button>
        <button type="button" className="btn btn-ghost" onClick={cancel}>Cancel</button>
      </div></div>
      {error}
    </form>
  )
  return place(<button type="button" className="btn" disabled={busy} onClick={go}><Icon name="bolt" size={14} />{busy ? 'Starting…' : 'Continue with ChatGPT'}</button>, error)
}

/** Sign in to Grok for Mimir with a device code: open the page, confirm the code, and this notices when it's done. */
function SignInGrok({ onDone }: { onDone: () => void }) {
  const [login, setLogin] = useState<{ url: string; code: string } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!login) return
    const t = setInterval(() => invoke<Record<string, Account>>('accounts').then(a => { if (a?.grok?.signed_in) { setLogin(null); onDone() } }, () => {}), 3000)
    return () => clearInterval(t)
  }, [login]) // eslint-disable-line react-hooks/exhaustive-deps
  const go = () => {
    setBusy(true); setErr(null)
    invoke<{ url: string; code: string }>('grok_login').then(l => { setLogin(l); if (inDesktop) localInvoke('open_url', { url: l.url }).catch(() => {}) }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  if (login) return (
    <span className="connect-wait" role="status"><span className="typing" aria-hidden="true"><i /><i /><i /></span>
      Confirm <code>{login.code}</code> at <a href={login.url} target="_blank" rel="noreferrer" onClick={openLink}>{(() => { try { return new URL(login.url).hostname } catch { return login.url } })()}</a>…
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => { setLogin(null); invoke('grok_login_cancel').catch(() => {}) }}>Cancel</button></span>
  )
  return <>
    <button type="button" className="btn" disabled={busy} onClick={go}><Icon name="bolt" size={14} />{busy ? 'Starting…' : 'Sign in to Grok'}</button>
    {err && <span className="error" style={{ margin: 0 }}>{err}</span>}
  </>
}

/** Model field with the account's models as suggestions. Warns about models that can't use tools. */
function ModelInput({ provider, baseUrl, value, onChange, signedIn = true }: { provider: string; baseUrl?: string; value: string; onChange: (v: string) => void; signedIn?: boolean }) {
  const [models, setModels] = useState<Model[]>([])
  useEffect(() => {
    setModels([])
    if (provider === 'claude' || provider === 'codex' || provider === 'grok' || (provider === 'chatgpt' && !signedIn)) return
    let live = true
    invoke<Model[]>('models', { provider, baseUrl: baseUrl?.trim() || null }).then(m => live && setModels(m ?? []), () => {})
    return () => { live = false }
  }, [provider, baseUrl, signedIn])
  const picked = models.find(m => m.id === value.trim())
  return (
    <>
      <input className="input" list={'models-' + provider} placeholder={MODEL_HINT[provider]} value={value} onChange={e => onChange(e.target.value)} aria-label="Model" />
      <datalist id={'models-' + provider}>{models.map(m => <option key={m.id} value={m.id} label={m.tools === false ? "can't use tools" : m.name !== m.id ? m.name : undefined} />)}</datalist>
      {picked?.tools === false && <span className="hint warn"><Icon name="alert" size={13} />This model can't use tools: it can chat, but it can't remember things, set routines or browse. Models like qwen3 or llama3.1 can.</span>}
    </>
  )
}

/** "What do you want off your plate?": each answer is a template bot (with its routines) and library skills; one
 *  without a template teaches the first bot its skills. */
const PLATE: { id: string; label: string; template?: string; skills: string[] }[] = [
  { id: 'brief', label: 'A morning briefing', template: 'brief', skills: ['morning-briefing'] },
  { id: 'tasks', label: 'Remembering my to-dos', template: 'tasks', skills: ['follow-up-tracker'] },
  { id: 'inbox', label: 'Keeping up with email', skills: ['inbox-triage'] },
  { id: 'meetings', label: 'Meeting prep and notes', skills: ['meeting-prep', 'meeting-follow-up'] },
  { id: 'research', label: 'Looking things up', template: 'research', skills: ['research-brief', 'summarize-link'] },
  { id: 'watch', label: 'Watching pages and prices', template: 'watch', skills: ['topic-watch', 'price-watch'] },
  { id: 'review', label: 'A weekly look back', template: 'review', skills: ['weekly-review'] },
]
/** What a library skill needs that isn't connected yet, in words. */
const NEEDS: Record<string, string> = { email_search: 'your email (Settings → Email)', calendar_list: 'your calendar link (Meetings)', meeting_read: 'a transcribed meeting (Meetings)' }

const ROLE_IDEAS = ["You're my personal assistant. Keep answers short.", 'Help me stay on top of tasks and follow-ups.', 'Research things for me and always link your sources.']

/** First run: pick an AI account that actually works, name the first bot, start talking. */
function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(1)
  const [accounts, setAccounts] = useState<Record<string, Account> | null>(null)
  const [provider, setProvider] = useState('claude')
  const [model, setModel] = useState('')
  const [key, setKey] = useState('')
  const [test, setTest] = useState<{ state: 'idle' | 'running' | 'ok' | 'fail'; text?: string }>({ state: 'idle' })
  const [name, setName] = useState('Mimir')
  const [role, setRole] = useState('')
  // Your first bot's creature: the one bot #1 has, until you change it.
  const [look, setLook] = useState<Look | null>(null)
  const [styling, setStyling] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  // What they want off their plate (up to three), and which of the suggestions that makes they turned off.
  const [plate, setPlate] = useState<string[]>([])
  const [skip, setSkip] = useState<string[]>([])
  const [library, setLibrary] = useState<LibrarySkill[]>([])
  const [busy, setBusy] = useState(false)
  // Suggestions already set up: trying again after an error doesn't make a second copy.
  const made = useRef(new Set<string>())
  useEffect(() => { invoke<LibrarySkill[]>('skill_library', { chatId: LOCAL }).then(setLibrary, () => {}) }, [])
  const lib = (slug: string) => library.find(k => k.slug === slug)
  const picked = PLATE.filter(p => plate.includes(p.id) && !skip.includes(p.id))

  const load = () =>
    invoke<Record<string, Account>>('accounts').then(a => {
      setAccounts(a)
      // Preselect the first account that's ready.
      const ready = ACCOUNT_ORDER.find(k => a[k]?.installed && a[k]?.signed_in)
      if (ready) pick(ready, a)
    }, e => setErr(String(e)))
  useEffect(() => { load() }, [])
  const check = () => { setAccounts(null); load() }
  // A local model server's first model that can use tools is the obvious pick.
  const pick = (k: string, a = accounts) => {
    setProvider(k); setTest({ state: 'idle' }); setKey('')
    setModel(k === 'local' ? (a?.local?.models?.find(m => m.tools !== false) ?? a?.local?.models?.[0])?.id ?? '' : '')
  }

  const runTest = async () => {
    setTest({ state: 'running' }); setErr(null)
    try {
      if (KEYED[provider] && key.trim()) await invoke('set_api_key', { kind: provider, key: key.trim() })
      const r = await invoke<{ reply: string; seconds: number }>('test_account', { provider, model: model.trim() || null, baseUrl: provider === 'local' ? acc?.base : null })
      setTest({ state: 'ok', text: `Works — replied in ${r.seconds}s` })
    } catch (e) { setTest({ state: 'fail', text: String(e) }) }
  }

  const finish = async () => {
    setBusy(true); setErr(null)
    try {
      const bots = await invoke<Bot[]>('bots')
      const account = { provider, model: model.trim(), base_url: provider === 'local' ? acc?.base ?? '' : '' }
      await invoke('save_bot', { bot: { ...bots[0], ...account, name: name.trim() || 'Mimir', persona: role, look: look ? JSON.stringify(look) : bots[0].look ?? '' }, secrets: {} })
      // The starter team: each template a bot on the same account with its routines; skills go to that bot, or the first.
      for (const p of picked.filter(p => !made.current.has(p.id))) {
        const t = TEMPLATES.find(t => t.id === p.template)
        const id = t ? await invoke<number>('save_bot', { bot: { ...BLANK, ...account, name: t.name, persona: t.persona, look: JSON.stringify(randomLook()) }, secrets: {} }) : bots[0].id
        if (t) await addStarterRoutines(id, t.routines)
        for (const slug of p.skills) await invoke('skill_add', { chatId: LOCAL, botId: id, slug })
        made.current.add(p.id)
      }
      await invoke('onboarding_done')
      onDone()
    } catch (e) { setErr(String(e)); setBusy(false) }
  }

  const acc = accounts?.[provider]
  const ready = KEYED[provider] ? !!key.trim() || !!acc?.signed_in : !!(acc?.installed && acc.signed_in) && (provider !== 'local' || !!model.trim())
  return (
    <div className="center-screen">
      <div className="onboard fade-in" key={step}>
        <img className="mark" src="/logo.png" alt="" />
        <div className="progress" aria-label={`Step ${step} of 4`}>{[1, 2, 3, 4].map(i => <i key={i} className={i <= step ? 'on' : ''} />)}</div>

        {step === 1 && (
          <>
            <div>
              <h1>Which AI should your bots use?</h1>
              <p className="lead">Mimir runs on the AI subscription you already have. We never bill you for it.</p>
            </div>
            <div className="accounts" role="radiogroup" aria-label="AI account">
              {ACCOUNT_ORDER.map(k => {
                const a = accounts?.[k]
                const [label, tone] = !accounts ? ['Checking…', '']
                  : KEYED[k] ? (a?.signed_in ? ['Key saved', 'live'] : ['Bring your own', ''])
                  : k === 'local' ? (!a?.installed ? ['Not running', 'pending'] : !a.signed_in ? ['No models yet', 'pending'] : [`${a.server} · ${a.models?.length} model${a.models?.length === 1 ? '' : 's'}`, 'live'])
                  : !a?.installed ? ['Not installed', 'pending'] : !a.signed_in ? ['Not signed in', 'pending']
                  : [a.plan ? `${a.plan[0].toUpperCase()}${a.plan.slice(1)} plan` : 'Signed in', 'live']
                return (
                  <button key={k} role="radio" aria-checked={provider === k} className={'account' + (provider === k ? ' on' : '')} onClick={() => provider !== k && pick(k)}>
                    <span className="ac-mark">{ACCOUNTS[k].mark}</span>
                    <span className="ac-text"><b>{ACCOUNTS[k].name}</b><span>{ACCOUNTS[k].desc}</span></span>
                    <span className={'badge ' + tone}>{tone === 'live' && <Icon name="check" size={11} />}{label}</span>
                  </button>
                )
              })}
            </div>
            {accounts && acc && !KEYED[provider] && (!acc.installed || !acc.signed_in) && (
              <div className="howto"><Icon name="alert" /><span>{!acc.installed ? ACCOUNTS[provider].install : ACCOUNTS[provider].signin}</span>
                <button className="btn btn-sm" onClick={check}>Check again</button></div>
            )}
            {provider === 'grok' && acc?.installed && !acc.signed_in && <div className="connect-row"><SignInGrok onDone={check} /></div>}
            {provider === 'chatgpt' && accounts && !acc?.signed_in && <SignInChatGPT onDone={check} place={(control, below) => <><div className="connect-row">{control}</div>{below}</>} />}
            {provider === 'chatgpt' && acc?.signed_in && <p className="hint" style={{ margin: 0 }}>You’re using your ChatGPT plan ({acc.account}). Set how much Mimir may use each week in <a href="https://chatgpt.com/#settings" onClick={openLink}>ChatGPT’s settings</a>.</p>}
            {provider === 'openrouter' && !acc?.signed_in && inDesktop && !remoteServer() && (
              <div className="connect-row"><ConnectOpenRouter onDone={check} /><span className="hint">or paste a key below</span></div>
            )}
            {KEYED[provider] && <input className="input" type="password" placeholder={acc?.signed_in ? 'Key saved — paste a new one to replace it' : KEYED[provider]} value={key} onChange={e => setKey(e.target.value)} autoComplete="off" aria-label="API key" />}
            <ModelInput provider={provider} baseUrl={provider === 'local' ? acc?.base : undefined} value={model} onChange={setModel} signedIn={!!acc?.signed_in} />
            {provider === 'claude' && <p className="hint" style={{ margin: 0 }}>Runs <code>claude -p</code> with your own login. Anthropic may count use by other apps as extra usage.</p>}
            {provider === 'grok' && acc?.signed_in && <p className="hint" style={{ margin: 0 }}>Runs <code>grok -p</code> on your subscription, with only Mimir's tools.</p>}
            {provider === 'openrouter' && <p className="hint" style={{ margin: 0 }}>{acc?.signed_in ? 'Connected.' : <>Keys are at <a href="https://openrouter.ai/keys" onClick={openLink}>openrouter.ai/keys</a>.</>} OpenRouter bills you per message; the run log shows what each one cost.</p>}
            {provider === 'local' && acc?.signed_in && <p className="hint" style={{ margin: 0 }}>Runs on this computer through {acc.server}. Your chats and memories don't leave it.</p>}
            <div className="actions">
              <button className="btn" disabled={!ready || test.state === 'running'} onClick={runTest}><Icon name="bolt" size={14} />{test.state === 'running' ? 'Testing…' : 'Test connection'}</button>
              {test.text && <span className={test.state === 'ok' ? 'ok-note' : 'error'} style={{ margin: 0 }}>{test.text}</span>}
              <span className="grow" />
              <button className="btn btn-primary btn-lg" disabled={test.state !== 'ok'} onClick={() => setStep(2)}>Continue</button>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <div>
              <h1>What do you want off your plate?</h1>
              <p className="lead">Pick up to three. Your first bot suggests a starter team for them, and you choose what to keep.</p>
            </div>
            <div className="chips" role="group" aria-label="What you want help with">
              {PLATE.map(p => {
                const on = plate.includes(p.id)
                return <button key={p.id} className={'chip' + (on ? ' on' : '')} aria-pressed={on} disabled={!on && plate.length >= 3}
                  onClick={() => setPlate(on ? plate.filter(x => x !== p.id) : [...plate, p.id])}>{on && <Icon name="check" size={11} />}{p.label}</button>
              })}
            </div>
            <div className="actions">
              <button className="btn btn-ghost" onClick={() => setStep(1)}>Back</button>
              <span className="grow" />
              <button className="btn btn-primary btn-lg" onClick={() => setStep(3)}>{plate.length ? 'Continue' : 'Skip'}</button>
            </div>
          </>
        )}

        {step === 3 && (
          <>
            <div className="page-head" style={{ margin: 0 }}>
              <button type="button" className="look-avatar" onClick={() => setStyling(s => !s)} aria-expanded={styling} title="Change how it looks">
                <Avatar id={1} name={name || '?'} size={52} alive look={look ?? undefined} mood={name.trim() ? 'listening' : 'idle'} beat={name.length} />
                <span className="look-edit"><Icon name="pencil" size={11} /></span>
              </button>
              <div><h1 style={{ margin: 0 }}>Name your first bot</h1><p className="lead">You can add more later, each with its own job. Tap it to change how it looks.</p></div>
            </div>
            {styling && <LookPicker id={1} look={look ?? {}} onChange={setLook} />}
            <label className="field-label"><span>Name</span><input className="input" value={name} onChange={e => setName(e.target.value)} autoFocus /></label>
            <label className="field-label"><span>What should it focus on? <span style={{ fontWeight: 400, color: 'var(--text-3)' }}>Optional</span></span>
              <textarea className="textarea" placeholder="Describe its job in a sentence or two." value={role} onChange={e => setRole(e.target.value)} /></label>
            <div className="chips">{ROLE_IDEAS.map(r => <button key={r} className={'chip' + (role === r ? ' on' : '')} onClick={() => setRole(r)}>{r}</button>)}</div>
            <div className="actions">
              <button className="btn btn-ghost" onClick={() => setStep(2)}>Back</button>
              <span className="grow" />
              <button className="btn btn-primary btn-lg" disabled={!name.trim()} onClick={() => setStep(4)}>Continue</button>
            </div>
          </>
        )}

        {step === 4 && (
          <>
            <div>
              <h1>{name.trim() || 'Your bot'} is ready</h1>
              <p className="lead">{plate.length ? `${name.trim() || 'It'} suggests a starter team. Untick what you don’t want; change any of it later.` : 'Tell it about yourself. Everything it remembers shows up in Memory, where you can correct or forget it.'}</p>
            </div>
            {plate.length > 0 ? (
              <ul className="tips plan" aria-label="Starter team">
                {PLATE.filter(p => plate.includes(p.id)).map(p => {
                  const on = !skip.includes(p.id)
                  const t = TEMPLATES.find(t => t.id === p.template)
                  const needs = [...new Set(p.skills.flatMap(slug => lib(slug)?.missing['1'] ?? []))].map(n => NEEDS[n]).filter(Boolean)
                  return (
                    <li key={p.id} className={on ? '' : 'off'}>
                      <input type="checkbox" checked={on} aria-label={p.label} onChange={() => setSkip(on ? [...skip, p.id] : skip.filter(x => x !== p.id))} />
                      <span><b>{t ? `New bot: ${t.name}` : `${name.trim() || 'Your bot'} takes on ${p.label.toLowerCase()}`}</b>
                        <small>{[...(t?.routines.map(r => `${schedule(r.cron)}: ${r.prompt}`) ?? []), `Skills: ${p.skills.map(slug => lib(slug)?.name ?? slug).join(', ')}`].join(' · ')}</small>
                        {needs.length > 0 && <small className="warn">Works once it has {needs.join(' and ')}.</small>}</span>
                    </li>
                  )
                })}
              </ul>
            ) : <ul className="tips">
              <li><Icon name="chat" />“Remember that I prefer short answers.”</li>
              <li><Icon name="clock" />“Every weekday at 8:30, remind me of my open tasks.”</li>
              <li><Icon name="telegram" />Connect Telegram, Slack or Discord later from the bot’s page.</li>
            </ul>}
            <div className="actions">
              <button className="btn btn-ghost" onClick={() => setStep(3)}>Back</button>
              <span className="grow" />
              <button className="btn btn-primary btn-lg" disabled={busy} onClick={finish}>{busy ? 'Setting up…' : picked.length ? 'Set them up and start' : 'Start chatting'}</button>
            </div>
          </>
        )}
        {err && <p className="error">{err}</p>}
      </div>
    </div>
  )
}

// ---------------- main window ----------------

const TABS: { id: Tab; label: string; icon: string; hint: string }[] = [
  { id: 'chat', label: 'Chat', icon: 'chat', hint: `Chat  ${shortcut('1')}` }, { id: 'memory', label: 'Memory', icon: 'memory', hint: `Memory  ${shortcut('2')}` },
  { id: 'skills', label: 'Skills', icon: 'bolt', hint: `Skills  ${shortcut('3')}` },
  { id: 'routines', label: 'Routines', icon: 'clock', hint: `Routines  ${shortcut('4')}` }, { id: 'activity', label: 'Activity', icon: 'activity', hint: `Activity  ${shortcut('5')}` },
]

function Main() {
  const bots = useLoad(() => invoke<Bot[]>('bots'), [])
  const statuses = useLoad(() => invoke<Status[]>('statuses'), [], 2000)
  // A bot another bot just made shows up in the statuses first: then the list reloads.
  useEffect(() => {
    if (bots.data && statuses.data?.some(s => !bots.data!.some(b => b.id === s.bot_id))) bots.reload()
  }, [statuses.data, bots.data]) // eslint-disable-line react-hooks/exhaustive-deps
  // What each bot is working on right now (a reply, a routine, a task from another bot, a memory tidy-up).
  const busy = useLoad(() => invoke<Busy[]>('busy'), [], 1500)
  const chats = useLoad(() => invoke<Chat[]>('chats'), [])
  const pending = useLoad(() => invoke<Approval[]>('approvals'), [], 1500)
  const inbox = useLoad(() => invoke<Inbox>('inbox'), [], 3000)
  // Bots that aren't working (an account or a messaging app that won't connect) need you too.
  const troubled = (statuses.data ?? []).flatMap(s => { const t = troubleOf(s); return t ? [{ id: s.bot_id, text: t }] : [] })
  const needs = inboxCount(inbox.data) + troubled.length
  const [chatId, setChatId] = useState<number>(LOCAL)
  const [botId, setBotId] = useState<number>(1) // for the bot page; 0 = new bot
  const [incognito, setIncognito] = useState(false)
  // Incognito ends when you go to another chat.
  useEffect(() => setIncognito(false), [chatId])
  // A link to the live computer (a bot asking you to take over) opens it: #computer.
  const [view, setView] = useState<View>(() => location.hash === '#computer' ? 'computer' : 'chat')
  useEffect(() => {
    const go = () => { if (location.hash === '#computer') { setView('computer'); history.replaceState(null, '', location.pathname + location.search) } }
    go()
    window.addEventListener('hashchange', go)
    return () => window.removeEventListener('hashchange', go)
  }, [])
  // Tell the server where its web app is reached, for links in messages (only a server keeps it).
  useEffect(() => {
    const at = inDesktop ? remoteServer()?.url : location.origin
    if (at && (!inDesktop || remoteServer())) invoke('public_url', { url: at }).catch(() => {})
  }, [])
  // Content drifts in from the side of the tab you moved to (left or right); other views just rise.
  const lastTab = useRef(0)
  const tabIndex = TABS.findIndex(t => t.id === view)
  const dx = useMemo(() => (tabIndex < 0 ? 0 : Math.sign(tabIndex - lastTab.current) * 14), [view]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (tabIndex >= 0) lastTab.current = tabIndex }, [tabIndex])
  // The sidebar's selection is one highlight that glides to the chosen bot or chat.
  const railRef = useRef<HTMLDivElement>(null)
  const railOn = useRef<HTMLElement | null>(null)
  useLayoutEffect(() => {
    const el = railRef.current
    if (!el) return
    const on = el.querySelector<HTMLElement>('.rail-item.on')
    el.classList.toggle('has-glide', !!on)
    if (!on) return
    // Glide only when the selection changes; when the list itself shifts (bots loading), jump.
    const jump = railOn.current === null || railOn.current === on
    railOn.current = on
    if (jump) el.classList.add('measuring')
    el.style.setProperty('--gy', `${on.offsetTop}px`)
    el.style.setProperty('--gh', `${on.offsetHeight}px`)
    if (jump) requestAnimationFrame(() => el.classList.remove('measuring'))
  })

  const botList = bots.data ?? []
  setLooks(botList) // every avatar of a bot shows the creature you chose for it
  const botOf = (id: number) => botList.find(b => b.id === id) ?? { id, name: `Bot ${id}` }
  const statusOf = (id: number) => statuses.data?.find(s => s.bot_id === id)
  const all = chats.data ?? []
  const list = all.some(c => c.chat_id === LOCAL) ? all : [{ chat_id: LOCAL, title: LOCAL_NAME, memories: 0, channel: 'local' }, ...all]
  const chat = list.find(c => c.chat_id === chatId) ?? list[0]
  const inChat = view !== 'settings' && view !== 'bot' && view !== 'inbox' && view !== 'computer' && view !== 'meetings'
  const computer = useLoad(() => invoke<ComputerStatus>('computer'), [], 20000)
  // A meeting being transcribed (this computer), for the sidebar's dot.
  const meetingNow = useLoad(() => inDesktop ? localInvoke<{ recording: boolean }>('meeting', { action: 'status' }) : Promise.resolve({ recording: false }), [], 3000)
  const recording = !!meetingNow.data?.recording
  const openBot = (id: number) => { setBotId(id); setView('bot'); setMenu(null) }
  // From elsewhere (Needs you): the chat as a whole, not the bot view you had open before.
  const openChat = (id: number, v: Tab) => { setChatId(id); setViaBot(false); setView(v) }
  // Clicking a bot opens a chat with it (on this computer); its settings are a right-click (or the gear by the
  // composer's bot picker) away.
  const [chatBot, setChatBot] = useState<number | null>(null)
  const [viaBot, setViaBot] = useState(false)
  const talkTo = (id: number) => { setChatBot(id); setChatId(LOCAL); setIncognito(false); setViaBot(true); setView('chat'); setMenu(null) }
  const botChat = viaBot && view === 'chat' && chat.chat_id === LOCAL
  const [menu, setMenu] = useState<{ x: number; y: number; id: number } | null>(null)
  // A bot pointing at a setting ("Settings → Email") opens Settings searched for it.
  const [find, setFind] = useState('')
  useEffect(() => { if (view !== 'settings') setFind('') }, [view])
  const openSettings = (q: string) => { setFind(q); setView('settings') }

  // ⌘F: on the Settings page it searches settings; anywhere else, memory.
  const findIt = () => {
    const box = document.querySelector<HTMLInputElement>('.settings-search input')
    if (box) { box.focus(); box.select(); return }
    setView('memory')
    setTimeout(() => document.querySelector<HTMLInputElement>('.search input')?.focus(), 50)
  }
  // The macOS menu bar sends its commands here (same actions as the shortcuts below).
  useEffect(() => {
    if (!inDesktop) return
    let off: (() => void) | undefined
    import('@tauri-apps/api/event').then(({ listen }) => listen<string>('menu', e => {
      const id = e.payload
      if (id === 'new_bot') { setBotId(0); setView('bot') }
      else if (id === 'search') findIt()
      else if (['settings', 'inbox', 'chat', 'memory', 'skills', 'routines', 'activity', 'meetings'].includes(id)) setView(id as View)
    })).then(u => { off = u }, () => {})
    return () => off?.()
  }, [])
  // ⌘ on macOS, Ctrl elsewhere: 1–5 sections, 0 Needs you, comma Settings, N new bot, F search memory.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return
      const tab = TABS[Number(e.key) - 1]
      if (tab) setView(tab.id)
      else if (e.key === ',') setView('settings')
      else if (e.key === '0') setView('inbox')
      else if (e.key === 'n') { setBotId(0); setView('bot') }
      else if (e.key === 'f') findIt()
      else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  const chatName = (c: Chat) => c.chat_id === LOCAL ? LOCAL_NAME : c.title || `Chat ${c.chat_id}`
  const ch = CHANNEL[chat.chat_id === LOCAL ? 'local' : chat.channel] ?? CHANNEL.local

  return (
    <Bots.Provider value={botOf}>
    <div className="app">
      <aside className="rail" aria-label="Bots and chats">
        <div className="rail-top" data-tauri-drag-region />
        <div className="rail-scroll" ref={railRef}>
          <span className="rail-glide" aria-hidden="true" />
          <div className="brand"><img src="/logo.png" alt="" />Mimir{remoteServer() && <span className="where" title={remoteServer()!.url}>server</span>}</div>
          <button className={'rail-item inbox-item' + (view === 'inbox' ? ' on' : '')} onClick={() => setView('inbox')}>
            <span className="ri-glyph"><Icon name="inbox" size={15} /></span>
            <span className="ri-text"><span className="ri-name">Needs you</span><span className="ri-sub">{needs ? 'Waiting for you' : 'All clear'}</span></span>
            {needs > 0 && <span className="badge count" key={needs}>{needs}</span>}
          </button>
          {inDesktop && (
            <button className={'rail-item' + (view === 'meetings' ? ' on' : '')} onClick={() => setView('meetings')}>
              <span className="ri-glyph"><Icon name="mic" size={15} /></span>
              <span className="ri-text"><span className="ri-name">Meetings</span><span className="ri-sub">{recording ? 'Transcribing…' : 'Transcribe a call'}</span></span>
              {recording && <span className="rec-dot" aria-label="Recording" />}
            </button>
          )}
          {computer.data?.enabled && (
            <button className={'rail-item' + (view === 'computer' ? ' on' : '')} onClick={() => setView('computer')}>
              <span className="ri-glyph"><Icon name="laptop" size={15} /></span>
              <span className="ri-text"><span className="ri-name">Computer</span><span className="ri-sub">{computer.data.running ? 'Watch or take over' : 'Starting…'}</span></span>
            </button>
          )}
          <div className="rail-label">Bots<button title={`New bot  ${shortcut('N')}`} aria-label="New bot" onClick={() => openBot(0)}><Icon name="plus" size={14} /></button></div>
          {botList.map(b => {
            const st = statusOf(b.id)
            const on = online(st)
            const p = presenceOf(st)
            return (
              <button key={b.id} className={'rail-item' + ((view === 'bot' && botId === b.id) || (botChat && (chatBot ?? botList[0]?.id) === b.id) ? ' on' : '')}
                onClick={() => talkTo(b.id)} title={`Chat with ${b.name} · right-click for settings`}
                onContextMenu={e => {
                  e.preventDefault()
                  // From the keyboard there's no pointer: open it by the row.
                  const r = e.currentTarget.getBoundingClientRect()
                  setMenu(e.clientX || e.clientY ? { x: e.clientX, y: e.clientY, id: b.id } : { x: r.left + 24, y: r.bottom, id: b.id })
                }}
                onKeyDown={e => {
                  // The menu key and Shift+F10 open the menu (not every browser turns them into a contextmenu event).
                  if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
                    e.preventDefault()
                    const r = e.currentTarget.getBoundingClientRect()
                    setMenu({ x: r.left + 24, y: r.bottom, id: b.id })
                  }
                }}>
                {(() => {
                  // Waiting on you beats working: the reply (or routine) can't go on until you answer.
                  const asks = (pending.data ?? []).some(a => a.bot_id === b.id)
                  const w: Busy | undefined = asks ? { bot_id: b.id, chat_id: 0, trigger: 'ask', step: 'Waiting for your OK' } : busy.data?.find(x => x.bot_id === b.id)
                  return <>
                    <Avatar id={b.id} name={b.name} presence={p} mood={w ? busyMood(w) : undefined} />
                    <span className="ri-text"><span className="ri-name">{b.name}</span>
                      <span className={'ri-sub' + (w ? ' busy' : '')}>{w ? busyLabel(w)
                        : on.length ? on.map(c => CHANNEL[c.kind].name).join(' · ') : p === 'err' ? 'Needs attention' : p === 'wait' ? 'Connecting…' : `${LOCAL_NAME} only`}</span></span>
                  </>
                })()}
              </button>
            )
          })}
          <div className="rail-label">Chats</div>
          {list.map(c => {
            const cc = CHANNEL[c.chat_id === LOCAL ? 'local' : c.channel] ?? CHANNEL.local
            return (
              <button key={c.chat_id} className={'rail-item' + (inChat && !botChat && c.chat_id === chat.chat_id ? ' on' : '')}
                onClick={() => { setChatId(c.chat_id); setViaBot(false); if (!inChat) setView('chat') }}>
                <span className="ri-glyph"><Icon name={cc.icon} size={15} /></span>
                <span className="ri-text"><span className="ri-name">{chatName(c)}</span><span className="ri-sub">{cc.name}{c.guest ? ' · guest' : ''}</span></span>
                <span className="ri-count" title={`${c.memories} memories`}>{c.memories || ''}</span>
              </button>
            )
          })}
          {list.length <= 1 && <p className="rail-hint">Telegram, Slack and Discord chats appear here once you pair them.</p>}
        </div>
        <div className="rail-foot">
          <button className={'rail-item' + (view === 'settings' ? ' on' : '')} onClick={() => setView('settings')} title={`Settings  ${shortcut(',')}`}>
            <Icon name="settings" /> <span className="ri-text"><span className="ri-name">Settings</span></span>
          </button>
        </div>
      </aside>

      <main className="main" style={{ '--dx': `${dx}px` } as React.CSSProperties}>
        <header className="toolbar" data-tauri-drag-region>
          {inChat ? (
            <>
              <div className="toolbar-title" data-tauri-drag-region>
                <span className="ri-glyph" style={{ width: 26, height: 26, borderRadius: 7, display: 'grid', placeItems: 'center', background: 'var(--hover)', color: 'var(--text-2)' }}><Icon name={ch.icon} size={14} /></span>
                <span className="tt" data-tauri-drag-region><b>{botChat ? botOf(chatBot ?? botList[0]?.id ?? 1).name : chatName(chat)}</b><small>{ch.name} · {chat.memories} {chat.memories === 1 ? 'memory' : 'memories'}, shared by all your bots</small></span>
                {!botChat && chat.chat_id !== LOCAL && (
                  <button className={'btn btn-sm btn-ghost' + (chat.guest ? ' guest-on' : '')}
                    title={chat.guest ? 'Paired by someone else: bots here have their own memory, and none of your email, calendar, meetings, saved logins or tool servers. Click if this chat is yours.' : 'Click if someone else uses this chat (a partner, a housemate): bots here then get none of your email, calendar, meetings, saved logins or tool servers.'}
                    onClick={() => ask(chat.guest ? 'Make this chat yours? Bots here get your email, calendar, meetings, saved logins and tool servers.' : 'Treat this chat as a guest’s? Bots here lose your email, calendar, meetings, saved logins and tool servers.', chat.guest ? 'Make it mine' : 'Make it a guest’s')
                      .then(ok => { if (ok) void invoke('chat_guest', { chatId: chat.chat_id, guest: !chat.guest }).then(chats.reload, e => tell(String(e))) })}>
                    {chat.guest ? 'Guest' : 'Yours'}
                  </button>
                )}
              </div>
              <Segmented label="Sections" value={view as Tab} onChange={setView} options={TABS} />
            </>
          ) : (
            <div className="toolbar-title" data-tauri-drag-region>
              <span className="tt" data-tauri-drag-region><b>{view === 'settings' ? 'Settings' : view === 'inbox' ? 'Needs you' : view === 'computer' ? 'Computer' : view === 'meetings' ? 'Meetings' : botId === 0 ? 'New bot' : botOf(botId).name}</b></span>
            </div>
          )}
        </header>

        {view !== 'inbox' && <Approvals list={pending.data ?? []} chats={list} chatName={chatName} onDecided={() => { pending.reload(); inbox.reload() }} />}

        {view === 'chat' ? (
          <ChatView key={`chat-${chat.chat_id}-${incognito && chat.chat_id === LOCAL}-${botChat ? chatBot ?? botList[0]?.id : 'all'}`} only={botChat} chat={chat} bots={botList} onTurn={chats.reload}
            incognito={incognito && chat.chat_id === LOCAL} setIncognito={setIncognito} openBot={openBot} openSettings={openSettings}
            botId={chatBot ?? undefined} onBot={setChatBot} troubleOf={id => troubleOf(statusOf(id))} busy={busy.data ?? undefined} />
        ) : (
          // Keyed: each view starts at the top (the scroller used to keep the last view's position).
          <div className="page" key={`${view}-${chat.chat_id}-${botId}`}>
            <div className={'col fade-in' + (view === 'activity' || view === 'computer' ? ' wide' : '')}>
              {view === 'memory' && <MemoryView chatId={chat.chat_id} onChange={chats.reload} />}
              {view === 'skills' && <SkillsView chatId={chat.chat_id} />}
              {view === 'routines' && <RoutinesView chatId={chat.chat_id} bots={botList} />}
              {view === 'activity' && <ActivityView chatId={chat.chat_id} />}
              {view === 'settings' && <SettingsView onComputer={computer.reload} find={find} />}
              {view === 'computer' && <ComputerView bots={botList} chats={list} chatName={chatName} />}
              {view === 'meetings' && <MeetingsView bots={botList} onSent={talkTo} />}
              {view === 'inbox' && <InboxView data={inbox.data} troubled={troubled} reload={() => { inbox.reload(); pending.reload() }} chats={list} chatName={chatName} openBot={openBot} openChat={openChat} />}
              {view === 'bot' && (
                // Keyed on whether the bot has loaded too: right after "Create bot" the list hasn't caught up yet.
                <BotPage key={`${botId}:${botList.some(b => b.id === botId)}`} bot={botList.find(b => b.id === botId)} status={statusOf(botId)} canDelete={botList.length > 1}
                  // Only a new bot's page moves (to the bot it made): a slow save mustn't pull you back from another bot.
                  onSaved={id => { bots.reload(); statuses.reload(); if (botId === 0) setBotId(id) }}
                  onDeleted={() => { bots.reload(); if (chatBot === botId) { setChatBot(null); setViaBot(false) } setView('chat') }} onChat={() => talkTo(botId)} />
              )}
            </div>
          </div>
        )}
      </main>
      {menu && <BotMenu at={menu} name={botOf(menu.id).name} onChat={() => talkTo(menu.id)} onSettings={() => openBot(menu.id)} onClose={() => setMenu(null)} />}
    </div>
    </Bots.Provider>
  )
}

// ---------------- approvals ----------------

/** Let an item play its exit before the action that removes it: `leave(id, run)`, `leaving.has(id)`. */
function useLeave() {
  const [leaving, setLeaving] = useState<Set<number>>(new Set())
  const leave = (id: number, run: () => Promise<unknown>, ms = 240) => {
    setLeaving(s => new Set(s).add(id))
    setTimeout(() => run().finally(() => setLeaving(s => { const n = new Set(s); n.delete(id); return n })), ms)
  }
  return { leaving, leave }
}

/** One approval: what it does, and Deny / Always / Allow once. Shared by the floating banner and the inbox. */
function ApprovalCard({ a, where, leaving, decide, inline }: {
  a: Approval; where: string; leaving: boolean; decide: (id: number, approve: boolean, always?: 'tool' | 'site' | 'run') => void; inline?: boolean
}) {
  const botOf = useContext(Bots)
  const b = botOf(a.bot_id)
  const risk = approvalRisk(a)
  // Allowed the same thing a few times already: offer to stop asking.
  const learned = risk.always && (a.allowed_before ?? 0) >= 2
  return (
    <div className={'approval tone-' + risk.tone + (inline ? ' inline' : '') + (leaving ? ' leaving' : '')}>
      <Avatar id={b.id} name={b.name} size={32} mood="asking" />
      <div className="approval-main">
        <div className="ask">{b.name} {a.kind === 'takeover' ? 'needs you on the computer' : 'needs your OK'} · {where}</div>
        <div className={'what selectable' + (a.kind === 'plan' ? ' plan' : '')}>{a.summary}</div>
        <div className="risk"><Icon name={risk.icon} size={12} />{risk.label}</div>
        {learned && <div className="learned">You've allowed this {a.allowed_before} times. Stop asking?</div>}
        {a.parked && <div className="parked">From a routine. It runs when you approve; it waits {((w: string) => w.startsWith('in ') ? `for ${w.slice(3)}` : `until ${w}`)(ahead(a.expires_at))}.</div>}
      </div>
      {a.kind === 'takeover' ? <div className="choices">
        <button className="btn" onClick={() => decide(a.id, false)}>Can’t now</button>
        <button className="btn" onClick={() => decide(a.id, true)}>Done</button>
        <a className="btn btn-primary" href="#computer">Open the computer</a>
      </div> : <div className="choices">
        <button className="btn" onClick={() => decide(a.id, false)}>Deny</button>
        {risk.always && (
          <button className={'btn' + (learned ? ' btn-primary' : '')} onClick={() => decide(a.id, true, a.site ? 'site' : 'tool')}
            title={a.site ? `Stop asking before this on ${a.site}. Change it in Settings → Permissions.` : `Stop asking before ${toolLabel(a.tool).toLowerCase()}. Change it in Settings → Permissions.`}>
            {/* It allows the whole tool, not just this one call: say which. */}
            {a.site ? `Always on ${a.site}` : `Always: ${toolLabel(a.tool).toLowerCase()}`}
          </button>
        )}
        {a.kind === 'rule' && !a.parked && (
          <button className="btn" onClick={() => decide(a.id, true, 'run')} title="Allow this, and anything else like it while the bot finishes this reply.">All like this</button>
        )}
        <button className={'btn' + (learned ? '' : ' btn-primary')} onClick={() => decide(a.id, true)}>{a.kind === 'plan' ? 'Approve plan' : 'Allow once'}</button>
      </div>}
      {!a.parked && <Countdown until={a.expires_at} />}
    </div>
  )
}

/** Decide on approvals with the leave animation, then refresh. */
function useDecide(onDecided: () => unknown) {
  const [err, setErr] = useState<string | null>(null)
  const { leaving, leave } = useLeave()
  const decide = (id: number, approve: boolean, always?: 'tool' | 'site' | 'run') =>
    leave(id, () => invoke('decide', { id, approve, always: always ?? null }).then(() => { setErr(null); return onDecided() }, e => setErr(String(e))))
  return { err, leaving, decide }
}

/** Asks you can answer all at once: plain ones (and plans). Sending an email, making a bot, a saved login and anything
 *  asked after outside content stay one by one: those are the ones to read. */
const bulkable = (a: Approval) => !a.kind || a.kind === 'rule' || a.kind === 'plan'

function DecideAll({ list, decide }: { list: Approval[]; decide: (id: number, approve: boolean) => unknown }) {
  const many = list.filter(bulkable)
  if (many.length < 2) return null
  const rest = list.length - many.length
  const go = async (approve: boolean) => {
    if (!await ask(`${approve ? 'Allow' : 'Deny'} these ${many.length}?${rest ? ` The other ${rest} (an email to send, a new bot, a saved login, or asked after outside content) you answer one by one.` : ''}`, approve ? `Allow ${many.length}` : `Deny ${many.length}`, !approve)) return
    for (const a of many) decide(a.id, approve)
  }
  return (
    <div className="decide-all">
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => go(false)}>Deny all</button>
      <button type="button" className="btn btn-sm" onClick={() => go(true)}>Allow all{rest ? ` ${many.length}` : ''}</button>
    </div>
  )
}

/** Live asks float over the window. Parked ones (from routines) wait in the inbox instead. */
function Approvals({ list, chats, chatName, onDecided }: { list: Approval[]; chats: Chat[]; chatName: (c: Chat) => string; onDecided: () => void }) {
  const { err, leaving, decide } = useDecide(onDecided)
  const live = list.filter(a => !a.parked)
  if (!live.length) return null
  const where = (id: number) => { const c = chats.find(x => x.chat_id === id); return id === INCOGNITO ? 'Incognito chat' : c ? chatName(c) : `chat ${id}` }
  return (
    <section className="approvals" aria-live="polite" aria-label="Waiting for your approval">
      <DecideAll list={live} decide={decide} />
      {live.map(a => <ApprovalCard key={a.id} a={a} where={where(a.chat_id)} leaving={leaving.has(a.id)} decide={decide} />)}
      {err && <p className="error">{err}</p>}
    </section>
  )
}

type Hold = { key: string; bot_id: number; routine_id: number | null; routine?: string | null; until: number; why: string; cap: boolean }
/** One item per usage limit: every bot on that AI login waits for the same reset. */
const holdGroups = (hs: Hold[]) => Object.values(hs.reduce<Record<string, Hold[]>>((g, h) => { const k = h.cap || h.routine_id ? h.key : `limit:${h.until}`; (g[k] ??= []).push(h); return g }, {}))
type Inbox = { approvals: Approval[]; missed: Approval[]; failed: Run[]; holds?: Hold[]; suggestions: { chat_id: number; count: number }[]; unconfirmed?: { chat_id: number; count: number }[]; server_update?: { current: string; latest: string; how: string } | null }
const inboxCount = (i: Inbox | null) => i ? (i.server_update ? 1 : 0) + i.approvals.length + i.missed.length + i.failed.length + holdGroups(i.holds ?? []).length + i.suggestions.reduce((n, s) => n + s.count, 0) + (i.unconfirmed ?? []).reduce((n, s) => n + s.count, 0) : 0

/** A failed run in plain words, with the fix that most likely helps. */
function failure(error: string | null, status?: Run['status'], delivered?: string | null): { title: string; fix: 'bot' | 'chat' | 'routines' | null; fixLabel?: string } {
  if (status !== 'error' && delivered?.startsWith('not sent')) return { title: `A routine ran, but its result couldn't be sent ${delivered.replace(/^not sent to (\w+).*/, (_, k: string) => `to ${k[0].toUpperCase()}${k.slice(1)}`)}`, fix: 'chat', fixLabel: 'Read it in the chat' }
  if (status === 'missed') return { title: error ?? 'A routine didn’t run on time', fix: 'routines', fixLabel: 'See routines' }
  const e = (error ?? '').toLowerCase()
  if (/usage limit|rate limit|quota|429/.test(e)) return { title: 'Hit the AI plan\'s usage limit', fix: 'bot', fixLabel: 'Use another account or model' }
  if (/rejected|unauthor|401|logged in|log in|sign in|signed in|api key/.test(e)) return { title: 'The AI account isn\'t signed in or its key was rejected', fix: 'bot', fixLabel: 'Check the AI account' }
  if (/failed to start|not installed|no such file|not found/.test(e)) return { title: 'Couldn\'t start the AI', fix: 'bot', fixLabel: 'Check the AI account' }
  if (/error sending request|connect|connection refused|dns|unreachable/.test(e)) return { title: 'Couldn\'t reach the AI. Is it running?', fix: 'bot', fixLabel: 'Check the AI account' }
  if (/pick a model/.test(e)) return { title: 'No model chosen', fix: 'bot', fixLabel: 'Choose a model' }
  if (/timed out|timeout/.test(e)) return { title: 'Took too long and was stopped', fix: 'chat', fixLabel: 'Open the chat' }
  return { title: 'Something went wrong', fix: 'chat', fixLabel: 'Open the chat' }
}

function InboxView({ data, troubled, reload, chats, chatName, openBot, openChat }: {
  data: Inbox | null; troubled: { id: number; text: string }[]; reload: () => unknown; chats: Chat[]; chatName: (c: Chat) => string; openBot: (id: number) => void; openChat: (chat: number, view: Tab) => void
}) {
  const botOf = useContext(Bots)
  const { err, leaving, decide } = useDecide(reload)
  const gone = useLeave()
  const where = (id: number) => { const c = chats.find(x => x.chat_id === id); return id === INCOGNITO ? 'Incognito chat' : c ? chatName(c) : `chat ${id}` }
  const dismiss = (kind: 'run' | 'approval', id: number) => gone.leave(id, () => invoke('inbox_dismiss', { kind, id }).then(reload, e => { reload(); return tell(`Couldn't dismiss it: ${e}`) }))
  if (!data) return null
  if (!inboxCount(data) && !troubled.length) return (
    <Empty icon="check" title="All clear">Nothing needs you right now. Approvals, runs that didn't work, bots that can't connect, memory suggestions and memories to confirm show up here.</Empty>
  )
  return (
    <div className="inbox">
      {data.server_update && <>
        <div className="section-title" style={{ marginTop: 0 }}><h2>Update your server</h2></div>
        <div className="group">
          <div className="inbox-row">
            <span className="row-icon"><Icon name="server" size={15} /></span>
            <div className="inbox-main">
              <div className="inbox-title">Mimir {data.server_update.latest} is out (your server runs {data.server_update.current})</div>
              <div className="inbox-desc selectable"><Markdown text={data.server_update.how} /></div>
            </div>
            <div className="row-ctl"><a className="btn btn-sm" href={`https://github.com/currentbits/mimir/releases/tag/v${data.server_update.latest}`} onClick={openLink}>What’s new</a></div>
          </div>
        </div>
      </>}
      {troubled.length > 0 && <>
        <div className="section-title" style={{ marginTop: 0 }}><h2>Not working</h2><span className="count">{troubled.length}</span></div>
        <div className="group">{troubled.map(t => {
          const b = botOf(t.id)
          return (
            <div key={t.id} className="inbox-row">
              <Avatar id={b.id} name={b.name} size={28} mood="sad" />
              <div className="inbox-main"><div className="inbox-title">{b.name} can't connect</div><div className="inbox-desc">{t.text}</div></div>
              <div className="row-ctl"><button className="btn btn-sm" onClick={() => openBot(t.id)}>Settings</button></div>
            </div>
          )
        })}</div>
      </>}
      {data.approvals.length > 0 && <>
        <div className="section-title" style={troubled.length ? undefined : { marginTop: 0 }}><h2>Waiting for your OK</h2><span className="count">{data.approvals.length}</span><DecideAll list={data.approvals} decide={decide} /></div>
        <div className="inbox-list">{data.approvals.map(a => <ApprovalCard key={a.id} inline a={a} where={where(a.chat_id)} leaving={leaving.has(a.id)} decide={decide} />)}</div>
        {err && <p className="error">{err}</p>}
      </>}
      {data.suggestions.length > 0 && <>
        <div className="section-title"><h2>Memory suggestions</h2></div>
        <div className="group">{data.suggestions.map(s => (
          <Row key={s.chat_id} icon={<Icon name="wand" size={15} />} title={`${s.count} suggested ${s.count === 1 ? 'change' : 'changes'}`} desc={`For ${where(s.chat_id)}. Nothing changes until you accept.`}>
            <button className="btn btn-sm" onClick={() => openChat(s.chat_id, 'memory')}>Review</button>
          </Row>
        ))}</div>
      </>}
      {(data.unconfirmed ?? []).length > 0 && <>
        <div className="section-title"><h2>Memories to confirm</h2></div>
        <div className="group">{data.unconfirmed!.map(s => (
          <Row key={s.chat_id} icon={<Icon name="shield" size={15} />} title={`${s.count} ${s.count === 1 ? 'memory' : 'memories'} to check`}
            desc={`For ${where(s.chat_id)}. Saved after a bot read outside content, so your bots don't use ${s.count === 1 ? 'it' : 'them'} until you say ${s.count === 1 ? 'it’s' : 'they’re'} right.`}>
            <button className="btn btn-sm" onClick={() => openChat(s.chat_id, 'memory')}>Review</button>
          </Row>
        ))}</div>
      </>}
      {(data.holds ?? []).length > 0 && <>
        <div className="section-title"><h2>Paused</h2><span className="count">{holdGroups(data.holds!).length}</span></div>
        <div className="group">{holdGroups(data.holds!).map(hs => {
          const h = hs[0], b = botOf(h.bot_id)
          const names = hs.map(x => botOf(x.bot_id).name)
          const who = names.length > 1 ? `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}` : names[0]
          const what = h.routine ? `“${h.routine}”` : h.routine_id ? 'A routine' : `${who}’s routines`
          return (
            <div key={h.key} className="inbox-row">
              <Avatar id={b.id} name={b.name} size={28} still />
              <div className="inbox-main">
                <div className="inbox-title">{what} {h.routine_id ? 'waits' : 'wait'} until {fullTime(h.until)}</div>
                <div className="inbox-desc">{h.cap ? `${h.why[0].toUpperCase()}${h.why.slice(1)}.` : 'The AI plan reached its usage limit.'} {h.routine_id ? 'It runs' : 'They run'} then, once, and the run log says which times {h.routine_id ? 'it' : 'they'} skipped.</div>
              </div>
              <div className="row-ctl">
                <button className="btn btn-sm" title={h.cap ? 'Let it go over the cap for the rest of today' : 'If you raised the limit or switched account'} onClick={() => Promise.all(hs.map(x => invoke('hold_clear', { key: x.key }))).then(reload, e => tell(`Couldn't let it go on: ${e}`))}>{h.cap ? 'Let it go on today' : 'Run now'}</button>
                {!h.cap && <button className="btn btn-sm btn-ghost" onClick={() => openBot(h.bot_id)}>Use another account</button>}
              </div>
            </div>
          )
        })}</div>
      </>}
      {data.failed.length > 0 && <>
        <div className="section-title"><h2>Didn’t work</h2><span className="count">{data.failed.length}</span></div>
        <div className="group">{data.failed.map(r => {
          const f = failure(r.error, r.status, r.delivered)
          const b = botOf(r.bot_id)
          return (
            <div key={r.id} className={'inbox-row' + (gone.leaving.has(r.id) ? ' leaving' : '')}>
              <Avatar id={b.id} name={b.name} size={28} mood="sad" />
              <div className="inbox-main">
                <div className="inbox-title">{f.title}</div>
                <div className="inbox-desc">{b.name} · {RUN_TITLE[r.trigger]} in {where(r.chat_id)} · {ago(r.started_at)}{(r.repeats ?? 1) > 1 ? ` · ${r.repeats} times` : ''}</div>
                {r.status === 'error' && r.error?.match(/(Trying again at [^.]+\.|This bot's routines wait until [^;]+)/) && <div className="inbox-desc">{r.error.match(/(Trying again at [^.]+\.|This bot's routines wait until [^;]+)/)![1]}</div>}
                {r.delivered?.startsWith('not sent') && <details className="inbox-raw"><summary>Details</summary><code className="selectable">{r.delivered}</code></details>}
                {r.error && r.status !== 'missed' && <details className="inbox-raw"><summary>Details</summary><code className="selectable">{r.error}</code></details>}
              </div>
              <div className="row-ctl">
                {f.fix && <button className="btn btn-sm" onClick={() => f.fix === 'bot' ? openBot(r.bot_id) : openChat(r.chat_id, f.fix === 'routines' ? 'routines' : 'chat')}>{f.fixLabel}</button>}
                <button className="btn btn-sm btn-ghost" onClick={() => dismiss('run', r.id)}>Dismiss</button>
              </div>
            </div>
          )
        })}</div>
      </>}
      {data.missed.length > 0 && <>
        <div className="section-title"><h2>Missed</h2><span className="count">{data.missed.length}</span></div>
        <div className="group">{data.missed.map(a => {
          const b = botOf(a.bot_id)
          return (
            <div key={a.id} className={'inbox-row' + (gone.leaving.has(a.id) ? ' leaving' : '')}>
              <Avatar id={b.id} name={b.name} size={28} mood="sad" still />
              <div className="inbox-main">
                <div className="inbox-title">{a.summary}</div>
                <div className="inbox-desc">{b.name} asked in {where(a.chat_id)} {ago(a.created_at)}. Nobody answered in time, so it wasn’t done.</div>
              </div>
              <div className="row-ctl"><button className="btn btn-sm btn-ghost" onClick={() => dismiss('approval', a.id)}>Dismiss</button></div>
            </div>
          )
        })}</div>
      </>}
    </div>
  )
}

/** A draining bar: when it runs out, the request counts as denied. */
function Countdown({ until }: { until: number }) {
  const [now, setNow] = useState(() => Date.now() / 1000)
  useEffect(() => { const t = setInterval(() => setNow(Date.now() / 1000), 1000); return () => clearInterval(t) }, [])
  const left = Math.max(0, until - now)
  return <span className="timer" style={{ width: `${Math.min(100, (left / 300) * 100)}%` }} title={`Denied automatically in ${Math.floor(left / 60)}:${String(Math.round(left % 60)).padStart(2, '0')}`} />
}

// ---------------- chat ----------------

const STARTERS = ['Remember that I prefer short answers.', 'What do you know about me?', 'Every weekday at 8:30, remind me of my open tasks.']
/** "[Scheduled quiet check #3 "0 8 * * *"] Look at…" → its parts. */
const SCHEDULED = /^\[Scheduled (routine|quiet check) #(\d+) "([^"]+)"[^\]]*\]\s*([\s\S]*)$/

function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  const copy = () => copyText(text).then(() => { setDone(true); setTimeout(() => setDone(false), 1400) }, e => tell(`Couldn't copy: ${e}`))
  return (
    <button type="button" className={'msg-copy' + (done ? ' done' : '')} onClick={copy} aria-label={done ? 'Copied' : 'Copy reply'} title={done ? 'Copied' : 'Copy'}>
      <Icon name={done ? 'check' : 'copy'} size={13} />
    </button>
  )
}

/** A bot's right-click menu in the sidebar: chat with it, or its settings. Escape, a click elsewhere or leaving the
 * menu closes it; arrow keys move between its items. */
function BotMenu({ at, name, onChat, onSettings, onClose }: { at: { x: number; y: number }; name: string; onChat: () => void; onSettings: () => void; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    // Kept inside the window.
    el.style.left = `${Math.min(at.x, window.innerWidth - el.offsetWidth - 8)}px`
    el.style.top = `${Math.min(at.y, window.innerHeight - el.offsetHeight - 8)}px`
    el.querySelector<HTMLButtonElement>('button')?.focus()
    const away = (e: Event) => { if (!el.contains(e.target as Node)) onClose() }
    window.addEventListener('pointerdown', away, true)
    window.addEventListener('blur', onClose)
    return () => { window.removeEventListener('pointerdown', away, true); window.removeEventListener('blur', onClose) }
  }, [at.x, at.y]) // eslint-disable-line react-hooks/exhaustive-deps
  const onKey = (e: React.KeyboardEvent) => {
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])]
    const i = items.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'Escape' || e.key === 'Tab') { e.preventDefault(); onClose() }
    else if (e.key === 'ArrowDown') { e.preventDefault(); items[(i + 1) % items.length]?.focus() }
    else if (e.key === 'ArrowUp') { e.preventDefault(); items[(i - 1 + items.length) % items.length]?.focus() }
  }
  return (
    <div ref={ref} className="ctx-menu" role="menu" aria-label={name} onKeyDown={onKey} onContextMenu={e => e.preventDefault()}>
      <button role="menuitem" onClick={onChat}><Icon name="chat" size={14} />Chat with {name}</button>
      <button role="menuitem" onClick={onSettings}><Icon name="settings" size={14} />Settings</button>
    </div>
  )
}

function ChatView({ chat, bots, onTurn, incognito = false, setIncognito, openBot, openSettings, botId: chosen, onBot, troubleOf: trouble, only = false, busy: running = [] }: {
  chat: Chat; bots: Bot[]; onTurn: () => void; incognito?: boolean; setIncognito?: (on: boolean) => void; openBot?: (id: number) => void
  openSettings?: (find: string) => void
  // Opened from a bot in the sidebar: only that bot's conversation, not everyone's in this chat.
  only?: boolean
  // The bot to message, when the app chose it (a click on a bot in the sidebar); the picker reports changes back.
  botId?: number; onBot?: (id: number) => void
  // What's wrong with a bot, if anything (shown above the composer, with a way to its settings).
  troubleOf?: (id: number) => string | null
  // What bots are working on right now (to pick up a reply that started before this view opened).
  busy?: Busy[]
}) {
  const [own, setOwn] = useState<number>(bots[0]?.id ?? 1)
  // A bot that was deleted falls back to the first one.
  const botId = chosen !== undefined && bots.some(b => b.id === chosen) ? chosen : bots.some(b => b.id === own) ? own : bots[0]?.id ?? own
  const mine = only ? botId : undefined
  const log = useLoad(() => incognito ? Promise.resolve([] as Message[]) : invoke<Message[]>('messages', { chatId: chat.chat_id, botId: mine }), [chat.chat_id, incognito, mine], incognito ? 0 : 4000)
  // Incognito messages exist only here; leaving or switching it off drops them.
  const [secret, setSecret] = useState<Message[]>([])
  const cid = incognito ? INCOGNITO : chat.chat_id
  const botOf = useContext(Bots)
  const [text, setText] = useState('')
  const setBotId = (id: number) => { setOwn(id); onBot?.(id) }
  const [pending, setPending] = useState<string | null>(null)
  // The bot your message went to: the reply streams from it even if the picker moves on.
  const [to, setTo] = useState<number | null>(null)
  // A reply started before this view opened (you came back mid-reply, "Summarise with…"): shown as it's written too.
  // Not your own send's: the bots' list lags a moment behind it.
  const [sentHere, setSentHere] = useState(false)
  const replying = chat.chat_id === LOCAL && !incognito && !sentHere ? running.find(w => w.chat_id === LOCAL && w.trigger === 'message' && (!only || w.bot_id === botId))?.bot_id ?? null : null
  useEffect(() => { if (!pending && !running.some(w => w.chat_id === LOCAL && w.trigger === 'message')) setSentHere(false) }, [running, pending])
  const working = !!pending || replying !== null
  const replier = to ?? replying ?? botId
  // "/" lists the bot's skills: "/book-padel Thursday" asks it to use that one.
  const skillList = useLoad(() => incognito ? Promise.resolve([] as Skill[]) : invoke<Skill[]>('skills', { chatId: chat.chat_id }), [chat.chat_id, incognito], 30000)
  const slashing = /^\/[^\s]*$/.test(text)
  const skillHits = slashing ? (skillList.data ?? []).filter(k => k.status === 'active' && (k.bot_id === botId || k.bot_id === 0) && skillSlug(k.name).startsWith(text.slice(1).toLowerCase().replace(/_/g, '-'))).slice(0, 6) : []
  const [draft, setDraft] = useState('')
  const [step, setStep] = useState<string | null>(null)
  const [done, setDone] = useState<{ summary: string; ok: boolean; decision: string }[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [failed, setFailed] = useState<string | null>(null)
  // The bot's face reacts for a moment when a reply ends: pleased, or startled when you stopped it.
  const [react, setReact] = useState<Mood | null>(null)
  const flash = (m: Mood, ms = 1500) => { setReact(m); setTimeout(() => setReact(r => (r === m ? null : r)), ms) }
  const scroller = useRef<HTMLDivElement>(null)
  // Follow new messages only if you're at the bottom; otherwise offer a way down.
  const atBottom = useRef(true)
  const [unseen, setUnseen] = useState(false)
  const onScroll = () => {
    const el = scroller.current
    if (!el) return
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
    if (atBottom.current) setUnseen(false)
  }
  // All the way down: the composer floats over the thread's end, so scrolling a marker into view would hide the last message.
  const toEnd = (smooth = false) => scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: smooth ? 'smooth' : 'auto' })
  const box = useRef<HTMLTextAreaElement>(null)
  const local = chat.chat_id === LOCAL
  const msgs = incognito ? secret : log.data ?? []
  const conv = useLoad(() => invoke<{ tainted: boolean }>('conversation', { botId, chatId: cid }), [botId, cid], 4000)
  const [fresh, setFresh] = useState(false)
  const startFresh = () => invoke('new_conversation', { botId, chatId: chat.chat_id }).then(() => { conv.reload(); setFresh(true); setTimeout(() => setFresh(false), 2600) }, e => setErr(String(e)))
  // Messages that arrive while you watch animate in; the first load and your own send's results don't
  // (their bubbles are already on screen).
  const [animFrom, setAnimFrom] = useState(Infinity)
  const [chipsFrom, setChipsFrom] = useState(Infinity)
  // By message id, not count: a long chat shows only its last 200, so the count stops changing.
  const lastId = msgs.at(-1)?.id ?? 0
  const seen = useRef<number | null>(null)
  const ownSend = useRef(false)
  useLayoutEffect(() => {
    if (!log.data && !incognito) return
    if (seen.current !== null && lastId > seen.current) (ownSend.current ? setChipsFrom : setAnimFrom)(seen.current)
    ownSend.current = false
    seen.current = lastId
  }, [log.data, lastId])
  const anim = (m: Message) => (m.id > animFrom ? ' msg-in' : m.id > chipsFrom ? ' acts-in' : '')

  const shown = useRef(0)
  useLayoutEffect(() => {
    if (atBottom.current || pending) toEnd()
    else if (lastId > shown.current && shown.current > 0) setUnseen(true)
    shown.current = lastId
  }, [lastId, pending, draft.length]) // eslint-disable-line react-hooks/exhaustive-deps
  // While a reply is being written, show it as it streams in.
  // How long it's been working (shown after a few seconds).
  const [started, setStarted] = useState(0)
  const [now, setNow] = useState(0)
  useEffect(() => {
    if (!working) return
    const from = Math.floor(Date.now() / 1000)
    setStarted(from); setNow(from)
    const tick = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000)
    return () => clearInterval(tick)
  }, [working])
  useEffect(() => {
    if (!working) return
    const t = setInterval(() => invoke<{ draft: string | null; step: string | null; done: { summary: string; ok: boolean; decision: string }[] }>('live', { botId: replier, chatId: cid })
      .then(l => { setDraft(l?.draft ?? ''); setStep(l?.step ?? null); setDone(l?.done ?? []) }, () => {}), 150)
    return () => { clearInterval(t); setDraft(''); setStep(null); setDone([]) }
  }, [working, replier, cid])
  useLayoutEffect(() => {
    const t = box.current
    if (t) { t.style.height = 'auto'; t.style.height = `${Math.min(t.scrollHeight, 160)}px` }
  }, [text])

  // Photos pasted or dropped into the composer, shown as thumbnails until sent.
  const [photos, setPhotos] = useState<{ url: string; data: string; ext: string }[]>([])
  // A PDF or text file to send with the message (one at a time); the bot gets its text.
  const [doc, setDoc] = useState<{ name: string; data: string } | null>(null)
  const [dragging, setDragging] = useState(false)
  const addPhotos = (files: Iterable<File>) => {
    for (const f of files) {
      const ext = ({ 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' } as Record<string, string>)[f.type]
      if (!ext) {
        if (f.type === 'application/pdf' || f.type.startsWith('text/') || /\.(pdf|txt|md|csv|json|log)$/i.test(f.name)) {
          const r = new FileReader()
          r.onload = () => { const url = String(r.result); setDoc({ name: f.name, data: url.slice(url.indexOf(',') + 1) }) }
          r.readAsDataURL(f)
        }
        continue
      }
      const r = new FileReader()
      r.onload = () => { const url = String(r.result); setPhotos(p => [...p, { url, data: url.slice(url.indexOf(',') + 1), ext }].slice(0, 4)) }
      r.readAsDataURL(f)
    }
  }
  // Leaving an incognito chat mid-reply stops that reply (its result would have nowhere to go).
  const busy = useRef<{ botId: number; pending: boolean }>({ botId, pending: false })
  busy.current = { botId, pending: !!pending }
  useEffect(() => () => { if (incognito && busy.current.pending) invoke('stop', { botId: busy.current.botId, chatId: INCOGNITO }).catch(() => {}) }, [incognito])
  const sendIncognito = (t: string) => {
    setText(''); setPending(t); setErr(null); setFailed(null); atBottom.current = true
    const now = () => Math.floor(Date.now() / 1000)
    const asked: Message = { id: 0, bot_id: botId, role: 'user', content: t, ts: now() }
    invoke<string>('send_incognito', { botId, text: t, history: secret.map(m => [m.role, m.content]) })
      .then(reply => setSecret(s => [...s, { ...asked, id: s.length + 1 }, { id: s.length + 2, bot_id: botId, role: 'assistant', content: reply, ts: now() }]),
        e => /stopped/i.test(String(e)) ? setSecret(s => [...s, { ...asked, id: s.length + 1 }, { id: s.length + 2, bot_id: botId, role: 'assistant', content: '_Stopped._', ts: now() }]) : (setErr(String(e)), setFailed(t)))
      .finally(() => { ownSend.current = true; setPending(null); conv.reload() })
  }
  /** "@Name rest": the bot it names (longest name first, so "Follow-up keeper" beats "Follow") and the rest. */
  const mentioned = (t: string): [Bot, string] | null => {
    if (!t.startsWith('@') || bots.length < 2) return null
    const low = t.slice(1).toLowerCase()
    const b = [...bots].sort((x, y) => y.name.length - x.name.length)
      .find(b => low.startsWith(b.name.toLowerCase()) && /^([\s,:]|$)/.test(low.slice(b.name.length)))
    return b ? [b, t.slice(1 + b.name.length).replace(/^[\s,:]+/, '')] : null
  }
  const send = async (value = text) => {
    const at = incognito ? null : mentioned(value.trim())
    const t = at ? at[1].trim() : value.trim()
    if ((!t && !photos.length && !doc) || working) return
    if (incognito) return sendIncognito(t)
    // "@Researcher …" goes to that bot (and the picker follows). In one bot's own chat, its thread opens when the reply is in
    // (switching now would drop this view, and the reply with it).
    const to = at ? at[0].id : botId
    if (at && !only) setBotId(to)
    setTo(to); setSentHere(true)
    const shown = photos.length ? `📷 ${t || 'Photo'}` : doc ? `${t || '(I sent you a file.)'}\n\n[Attached: ${doc.name}. Its text follows` : t
    const attached = photos, file = doc
    setText(''); setPhotos([]); setDoc(null); setPending(shown); setErr(null); setFailed(null); atBottom.current = true
    const images = await Promise.all(attached.map(p => invoke<string>('save_media', { data: p.data, ext: p.ext }))).catch(e => { setErr(String(e)); return null })
    if (!images) { setPending(null); return }
    // The file's text goes with the message, marked as outside content (the conversation is marked too).
    const said = images.length ? `📷 ${t || '(I sent you a photo.)'}` : t
    const message = file ? await invoke<string>('read_document', { botId: to, chatId: LOCAL, name: file.name, data: file.data, text: said }).catch(e => { setErr(String(e)); setFailed(t); return null }) : said
    if (message === null) { setPending(null); return }
    invoke<string>('send_local', { botId: to, text: message, images })
      .then(() => flash('happy'), e => { if (/stopped/i.test(String(e))) flash('surprised', 900); else { setErr(String(e)); setFailed(t) } })
      // Swap the pending bubbles for the saved messages in one frame: no flash in between.
      .finally(() => { ownSend.current = true; log.reload().finally(() => { setPending(null); setTo(null) }); conv.reload(); onTurn(); if (only && to !== botId) onBot?.(to) })
  }
  const bubble = (m: Message, last = false) => {
    const sched = m.role === 'user' ? m.content.match(SCHEDULED) : null
    if (sched) return (
      <div className={'msg-trigger' + anim(m)} title={sched[4]}>
        <Icon name={sched[1] === 'quiet check' ? 'moon' : 'clock'} size={13} />
        <span>{sched[1] === 'quiet check' ? 'Quiet check' : 'Routine'} · {schedule(sched[3])} — {sched[4]}</span>
      </div>
    )
    if (m.role === 'user') return <div className={'msg-user' + anim(m)} title={fullTime(m.ts)}><Said text={m.content} /></div>
    const b = botOf(m.bot_id)
    return (
      <div className={'msg-bot' + anim(m)} style={{ '--pig': pigment(b.id) } as React.CSSProperties}>
        <Avatar id={b.id} name={b.name} size={26} still />
        <div>
          <div className="msg-head"><b>{b.name}</b><time title={fullTime(m.ts)}>{ago(m.ts)}</time><CopyButton text={m.content} /></div>
          <div className="msg-body"><Markdown text={m.content} /></div>
          {m.actions && <Actions actions={m.actions} chatId={chat.chat_id} runId={m.run_id} onChange={() => { log.reload(); onTurn() }} />}
          {last && setting && openSettings && !working && (
            <div className="msg-actions">
              <button className="btn btn-sm" onClick={() => openSettings(setting)}><Icon name="settings" size={13} />Open Settings → {setting}</button>
              <button className="btn btn-sm btn-ghost" onClick={() => send('It’s connected now. Please try again.')}><Icon name="retry" size={13} />Try again</button>
            </div>
          )}
        </div>
      </div>
    )
  }
  const me = botOf(botId)
  const them = botOf(replier)
  // The last reply says a setting isn't connected yet (bots name it in these words): a way there, and back.
  const setting = msgs.at(-1)?.role === 'assistant' ? msgs.at(-1)!.content.match(/Settings → (Email|Calendar|Web search|Browser)\b/)?.[1] : undefined

  return (
    <div className="page view-in" ref={scroller} onScroll={onScroll}>
      <div className="chat">
        <div className="thread selectable">
          {msgs.length === 0 && !working && (
            incognito ? (
              <Empty icon="incognito" title="Incognito chat">
                {me.name} won't see or save your memories here, and nothing from this chat is kept: not in history, memory or the run log. It ends when you switch it off or leave.
              </Empty>
            ) : local ? (
              <Empty icon={<Avatar id={me.id} name={me.name} size={72} alive beat={text.length} mood={text.trim() ? 'listening' : 'idle'} />} title={`Say hello to ${me.name}`} actions={<div className="starters">{STARTERS.map(s => <button key={s} className="starter" onClick={() => send(s)}>{s}</button>)}</div>}>
                Tell it about yourself or ask for something. Whatever it learns is saved to Memory, where you can see and change it.
              </Empty>
            ) : (
              <Empty icon={CHANNEL[chat.channel]?.icon ?? 'chat'} title="No messages yet">Messages with this {CHANNEL[chat.channel]?.name ?? ''} chat show up here.</Empty>
            )
          )}
          {msgs.map((m, i) => (
            <Fragment key={m.id}>
              {(i === 0 || dayLabel(m.ts) !== dayLabel(msgs[i - 1].ts)) && <div className="day-sep" role="separator"><span>{dayLabel(m.ts)}</span></div>}
              {bubble(m, i === msgs.length - 1)}
            </Fragment>
          ))}
          {working && (
            <>
              {pending && <div className="msg-user msg-in"><Said text={pending} /></div>}
              <div className="msg-bot msg-in" style={{ '--pig': pigment(them.id), animationDelay: '120ms' } as React.CSSProperties}>
                <Avatar id={them.id} name={them.name} size={26} alive mood={moodOf(step, draft)} />
                <div><div className="msg-head"><b>{them.name}</b>{draft && <time>writing…</time>}
                  {now - started >= 5 && <time className="elapsed">{now - started >= 60 ? `${Math.floor((now - started) / 60)}m ${(now - started) % 60}s` : `${now - started}s`}</time>}</div>
                  {(done.length > 0 || step) && (
                    <ol className="steps" aria-label="What it's doing">
                      {done.map((d, i) => <li key={i} className={d.ok ? 'ok' : 'failed'}><Icon name={d.ok ? 'check' : 'x'} size={11} />{d.summary}</li>)}
                      {step && <li className="now" key={'now' + done.length}><span className="typing" aria-hidden="true"><i /><i /><i /></span><span className="shine">{step}…</span></li>}
                    </ol>
                  )}
                  {draft ? <div className="msg-body"><Markdown text={draft} caret /></div> : !step && <span className="typing" aria-label="Thinking"><i /><i /><i /></span>}</div>
              </div>
            </>
          )}
          {err && (() => {
            const f = failure(err)
            return (
              <div className="send-failed">
                <div className="sf-text"><b>{f.title}</b>
                  <details className="inbox-raw"><summary>Details</summary><code className="selectable">{err}</code></details></div>
                {f.fix === 'bot' && openBot && <button className="btn btn-sm" onClick={() => openBot(botId)}>{f.fixLabel}</button>}
                {failed && <button className="btn btn-sm" onClick={() => send(failed)}><Icon name="retry" size={13} />Try again</button>}
              </div>
            )
          })()}
        </div>
        <div className="composer-wrap">
          {unseen && <button className="jump" onClick={() => { setUnseen(false); toEnd(true) }}><Icon name="down" size={13} />New message</button>}
          {incognito && <div className="guarded incog" role="status"><Icon name="incognito" size={14} /><span>Incognito: nothing here is saved.</span>
            {!pending && <button className="btn btn-sm btn-ghost" onClick={() => setIncognito?.(false)}>End</button>}</div>}
          {local && !incognito && conv.data?.tainted && !pending && (
            <div className="guarded" role="status">
              <Icon name="shield" size={14} />
              <span>{me.name} read outside content (a web page, a file, a tool’s result) in this conversation, so anything that changes your memory or data asks you first.</span>
              <button className="btn btn-sm btn-ghost" onClick={startFresh} title="Start a fresh conversation. Memories are kept.">Start fresh</button>
            </div>
          )}
          {(() => {
            const t = trouble?.(botId)
            return t && !pending && (
              <div className="guarded trouble" role="status"><Icon name="alert" size={14} /><span><b>{me.name} needs attention.</b> {t}</span>
                {openBot && <button className="btn btn-sm btn-ghost" onClick={() => openBot(me.id)}>Settings</button>}</div>
            )
          })()}
          {fresh && <div className="guarded ok" role="status"><Icon name="check" size={14} /><span>Fresh conversation. Your memories are kept.</span></div>}
          {local ? (
            <>
              {!incognito && doc && (
                <div className="attach-row">
                  <span className="attach-file"><Icon name="file" size={13} />{doc.name}
                    <button type="button" aria-label="Remove file" onClick={() => setDoc(null)}><Icon name="x" size={11} /></button></span>
                </div>
              )}
              {!incognito && photos.length > 0 && (
                <div className="attach-row">
                  {photos.map((p, i) => (
                    <span className="attach" key={i}><img src={p.url} alt="" />
                      <button type="button" aria-label="Remove photo" onClick={() => setPhotos(ps => ps.filter((_, j) => j !== i))}><Icon name="x" size={11} /></button></span>
                  ))}
                </div>
              )}
              {!incognito && bots.length > 1 && /^@[^\s]*$/.test(text) && (
                <div className="mention-pick" role="listbox" aria-label="Message which bot?">
                  {bots.filter(b => b.name.toLowerCase().startsWith(text.slice(1).toLowerCase())).map(b => (
                    <button key={b.id} type="button" role="option" aria-selected={false} onClick={() => { setText(`@${b.name} `); if (!only) setBotId(b.id); box.current?.focus() }}>
                      <Avatar id={b.id} name={b.name} size={18} still />{b.name}
                    </button>
                  ))}
                </div>
              )}
              {skillHits.length > 0 && (
                <div className="slash-menu" role="listbox" aria-label="Skills">{skillHits.map(k => (
                  <button key={k.id} type="button" role="option" aria-selected="false" onClick={() => { setText(`/${skillSlug(k.name)} `); box.current?.focus() }}>
                    <code>/{skillSlug(k.name)}</code><span className="hint">{k.description}</span>
                  </button>
                ))}</div>
              )}
              <form className={'composer' + (dragging ? ' dropping' : '') + (incognito ? ' incog' : '')} onSubmit={e => { e.preventDefault(); send() }}
                // Photos are saved to disk, so incognito doesn't take them.
                onDragOver={e => { if (!incognito && e.dataTransfer.types.includes('Files')) { e.preventDefault(); setDragging(true) } }}
                onDragLeave={() => setDragging(false)}
                onDrop={e => { e.preventDefault(); setDragging(false); if (!incognito) addPhotos(e.dataTransfer.files) }}>
                <span className="bot-pick" title={bots.length > 1 ? 'Choose a bot' : me.name}>
                  <Avatar id={me.id} name={me.name} size={28} alive beat={text.length}
                    mood={working ? moodOf(step, draft) : react ?? (err ? 'sad' : text.trim() ? 'listening' : 'idle')} />
                  {bots.length > 1 && (
                    <select value={botId} disabled={working} onChange={e => setBotId(Number(e.target.value))} aria-label="Bot to message">
                      {bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
                    </select>
                  )}
                </span>
                {openBot && !incognito && (
                  <button type="button" className="bot-settings" onClick={() => openBot(me.id)} title={`${me.name}'s settings`} aria-label={`${me.name}'s settings`}>
                    <Icon name="settings" size={14} />
                  </button>
                )}
                <textarea ref={box} rows={1} placeholder={incognito ? `Message ${me.name} incognito` : `Message ${me.name}`} value={text} onChange={e => setText(e.target.value)} disabled={working} autoFocus
                  onPaste={e => { if (incognito) return; const imgs = [...e.clipboardData.files].filter(f => f.type.startsWith('image/')); if (imgs.length) { e.preventDefault(); addPhotos(imgs) } }}
                  onKeyDown={e => {
                    if (e.key === 'Tab' && skillHits[0]) { e.preventDefault(); setText(`/${skillSlug(skillHits[0].name)} `); return }
                    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); send() }
                  }} />
                {!working && setIncognito && (
                  <button type="button" className={'incog-toggle' + (incognito ? ' on' : '')} aria-pressed={incognito} onClick={() => setIncognito(!incognito)}
                    title={incognito ? 'Back to the normal chat' : 'Incognito chat: no memory, nothing saved'}><Icon name="incognito" size={16} /></button>
                )}
                {working
                  ? <button type="button" className="send stop" aria-label="Stop" title="Stop" onClick={() => invoke('stop', { botId: replier, chatId: cid }).catch(e => setErr(`Couldn't stop it: ${e}`))}><span className="stop-square" /></button>
                  : <button className="send" aria-label="Send" disabled={!text.trim() && !photos.length}><Icon name="send" size={15} /></button>}
              </form>
              <p className="composer-note">{platform === 'mac' ? 'Return to send · Shift-Return for a new line' : 'Enter to send · Shift+Enter for a new line'}</p>
            </>
          ) : (
            <p className="composer-note">Reply from {CHANNEL[chat.channel]?.name ?? 'the app'}; this is a read-only view.</p>
          )}
        </div>
      </div>
    </div>
  )
}

// ---------------- memory ----------------

const KINDS: { id: Kind; title: string; empty: string }[] = [
  { id: 'convention', title: 'Preferences', empty: 'How you like things done.' },
  { id: 'knowledge', title: 'Facts', empty: 'People, places and details about your life.' },
  { id: 'task', title: 'Tasks', empty: 'Things you asked it to keep track of.' },
]
const KIND_SINGULAR: Record<Kind, string> = { convention: 'Preference', knowledge: 'Fact', task: 'Task' }

function MemoryView({ chatId, onChange }: { chatId: number; onChange: () => void }) {
  const rows = useLoad(() => invoke<EntryRow[]>('entries', { chatId }), [chatId])
  // How much of the standing memory (what every reply is given) is used.
  const standing = useLoad(() => invoke<{ used: number; budget: number; count: number }>('standing_memory', { chatId }), [chatId, rows.data?.map(r => `${r.id}:${r.status}:${r.pinned}:${r.background}`).join()])
  const [query, setQuery] = useState('')
  const [found, setFound] = useState<number[] | null>(null)
  const [history, setHistory] = useState<'now' | 'all'>('now')
  const [adding, setAdding] = useState<'one' | 'import' | null>(null)
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => {
    const q = query.trim()
    if (!q) return
    const t = setTimeout(() => invoke<{ id: number }[]>('search', { chatId, query: q }).then(r => setFound(r.map(e => e.id)), e => setErr(String(e))), 150)
    return () => clearTimeout(t)
  }, [query, chatId])

  const hits = query.trim() ? found : null
  const refresh = () => { rows.reload(); onChange() }
  const act = (p: Promise<unknown>) => p.then(() => { setErr(null); refresh() }, e => setErr(String(e)))

  const all = rows.data ?? []
  const byId = new Map(all.map(r => [r.id, r]))
  const inactive = all.filter(r => r.status !== 'active').length
  const visible = history === 'all' ? all : all.filter(r => r.status === 'active')
  // id → the entry it replaced, to show lineage on the successor.
  const replaced = new Map(all.filter(r => r.superseded_by).map(r => [r.superseded_by as number, r.id]))
  // Earlier versions of a memory, newest first.
  const earlier = (id: number) => {
    const out: EntryRow[] = []
    for (let p = replaced.get(id); p !== undefined && out.length < 50; p = replaced.get(p)) { const r = byId.get(p); if (r) out.push(r) }
    return out
  }
  const card = (r: EntryRow, showKind = false) => <MemoryCard key={r.id} r={r} versions={earlier(r.id)} act={act} chatId={chatId} showKind={showKind} />
  const pinned = history === 'now' ? visible.filter(r => r.pinned) : []
  const background = history === 'now' ? visible.filter(r => r.background) : []

  return (
    <>
      <div className="searchbar">
        <label className="search"><Icon name="search" size={15} /><input className="input" placeholder="Search memory" aria-label="Search memory" value={query} onChange={e => setQuery(e.target.value)} /></label>
        <Segmented size="sm" label="Which memories" value={history} onChange={setHistory}
          options={[{ id: 'now', label: 'Current' }, { id: 'all', label: `History${inactive ? ` · ${inactive}` : ''}` }]} />
        <TidyButton chatId={chatId} onDone={refresh} />
        <button className="btn btn-ghost" onClick={() => setAdding(a => a === 'import' ? null : 'import')} title="Bring in what another assistant remembers about you">Import</button>
        <button className="btn" onClick={() => setAdding(a => a === 'one' ? null : 'one')}><Icon name="plus" size={14} />Add</button>
      </div>
      {(err || rows.error) && <p className="error">{err || rows.error}</p>}
      {standing.data && standing.data.count > 0 && (() => {
        const pct = Math.round(standing.data.used * 100 / standing.data.budget)
        return <div className="standing" title="Every reply is given your pinned memories, your preferences and your most recent facts and tasks, within this budget. The rest is found by search when a message needs it.">
          <div className="standing-bar"><i style={{ width: `${Math.min(pct, 100)}%` }} className={pct >= 90 ? 'full' : ''} /></div>
          <span className="hint">Standing memory {pct}% full · {standing.data.count} {standing.data.count === 1 ? 'memory' : 'memories'} in every reply{pct >= 90 ? '; pin less, or move some to background' : ''}</span>
        </div>
      })()}
      <Suggestions chatId={chatId} entries={all} onChange={refresh} />
      <ConfirmAll chatId={chatId} entries={all} onDone={refresh} />
      <Learned chatId={chatId} entries={all} onChange={refresh} />
      <Files chatId={chatId} />
      {adding === 'one' && <div className="editor-card"><Editor submit="Save memory" onCancel={() => setAdding(null)} onSave={(kind, title, body) => act(invoke('remember', { chatId, kind, title, body, supersedes: null })).then(() => setAdding(null))} /></div>}
      {adding === 'import' && <ImportMemory chatId={chatId} onDone={() => { setAdding(null); refresh() }} />}

      {hits ? (
        <>
          <div className="section-title"><h2>Results</h2><span className="count">{hits.length}</span></div>
          {hits.length ? <div className="mem-list">{hits.map(id => byId.get(id)).filter((r): r is EntryRow => !!r).map(r => card(r, true))}</div>
            : <Empty icon="search" title="Nothing matches">Search matches whole words and the start of words.</Empty>}
        </>
      ) : visible.length === 0 ? (
        <Empty icon="memory" title="Nothing remembered yet">
          Tell a bot something about yourself, like “my dentist is Dr. Lee”. It’s saved here with where it was learned, and you can correct or forget it anytime.
        </Empty>
      ) : [{ id: 'pinned', title: 'Pinned' }, ...KINDS, { id: 'background', title: 'Background' }].map(k => {
        const items = k.id === 'pinned' ? pinned : k.id === 'background' ? background : visible.filter(r => r.kind === k.id && !pinned.includes(r) && !background.includes(r))
        return items.length ? (
          <section key={k.id}>
            <div className="section-title"><h2>{k.title}</h2><span className="count">{items.length}</span>
              {k.id === 'background' && <span className="hint">Not kept in mind, but found when a message needs it.</span>}</div>
            <div className="mem-list">{items.map(r => card(r))}</div>
          </section>
        ) : null
      })}
    </>
  )
}

function TidyButton({ chatId, onDone }: { chatId: number; onDone: () => void }) {
  const [state, setState] = useState<string | null>(null)
  const run = () => {
    setState('Reviewing…')
    invoke<number>('tidy', { chatId }).then(
      n => { setState(n ? `${n} suggestion${n === 1 ? '' : 's'}` : 'All tidy'); onDone(); setTimeout(() => setState(null), 4000) },
      e => setState(String(e)),
    )
  }
  return (
    <button className="btn btn-ghost" onClick={run} disabled={state === 'Reviewing…'} title="Review memory against recent conversations. Runs by itself once a day when the chat is quiet.">
      <Icon name="wand" size={14} />{state ?? 'Tidy up'}
    </button>
  )
}

/** Memories saved after reading outside content wait for "Looks right": all of them at once, after you've read them. */
function ConfirmAll({ chatId, entries, onDone }: { chatId: number; entries: EntryRow[]; onDone: () => void }) {
  const waiting = entries.filter(e => e.tainted && e.status === 'active')
  if (waiting.length < 2) return null
  const confirm = async () => {
    if (!await ask(`Mark all ${waiting.length} as right? They were saved after a bot read outside content (a page, a file, an email), so check they say what you meant: your bots then keep them in mind like anything you said.`, 'All look right')) return
    for (const e of waiting) await invoke('confirm_memory', { chatId, id: e.id }).catch(err => tell(String(err)))
    onDone()
  }
  return (
    <div className="confirm-all"><Icon name="shield" size={14} /><span>{waiting.length} memories wait for you to say they look right (marked “after reading outside content” below).</span>
      <button className="btn btn-sm" onClick={confirm}>All look right</button></div>
  )
}

const ACTION: Record<Suggestion['action'], [string, string]> = { add: ['Remember', 'live'], update: ['Update', 'pending'], forget: ['Forget', 'danger'], demote: ['Background', 'pending'] }

/** Proposed changes from the tidy-up. Nothing changes until accepted. */
function Suggestions({ chatId, entries, onChange }: { chatId: number; entries: EntryRow[]; onChange: () => void }) {
  const list = useLoad(() => invoke<Suggestion[]>('suggestions', { chatId }), [chatId], 10000)
  const [err, setErr] = useState<string | null>(null)
  const [checking, setChecking] = useState<Set<number>>(new Set())
  const { leaving, leave } = useLeave()
  const items = list.data ?? []
  if (!items.length) return null
  const target = (id: number | null) => entries.find(e => e.id === id)
  const check = (id: number) => {
    setChecking(c => new Set(c).add(id))
    invoke('check_suggestion', { chatId, id }).then(() => list.reload(), e => setErr(String(e)))
      .finally(() => setChecking(c => { const n = new Set(c); n.delete(id); return n }))
  }
  const decide = (id: number, accept: boolean) =>
    leave(id, () => invoke('decide_suggestion', { chatId, id, accept }).then(() => { setErr(null); return list.reload().then(onChange) }, e => setErr(String(e))))
  const all = async (accept: boolean) => {
    if (!accept && !await ask(`Dismiss all ${items.length} suggestions?`, 'Dismiss all', true)) return
    for (const s of items) await invoke('decide_suggestion', { chatId, id: s.id, accept }).catch(e => setErr(String(e)))
    list.reload().then(onChange)
  }
  return (
    <section className="suggest" aria-label="Suggested memory changes">
      <div className="suggest-head"><Icon name="wand" /><b>{items.length} suggested {items.length === 1 ? 'change' : 'changes'}</b><small>{items.every(s => s.reason.startsWith('Imported')) ? 'From what you imported.' : 'From recent conversations.'} Nothing changes until you accept.</small>
        {/* All at once. Ones drawn from outside content stay marked unconfirmed after accepting, as one by one. */}
        {items.length > 1 && <span className="suggest-all">
          <button className="btn btn-sm btn-ghost" onClick={() => all(false)}>Dismiss all</button>
          <button className="btn btn-sm" onClick={() => all(true)} title={items.some(s => s.tainted) ? 'The ones drawn from outside content stay marked unconfirmed until you say they look right' : undefined}>Accept all</button>
        </span>}</div>
      {items.map(s => {
        const t = target(s.entry_id)
        const [label, tone] = ACTION[s.action]
        return (
          <div className={'sugg ' + s.action + (leaving.has(s.id) ? ' leaving' : '')} key={s.id}>
            <span className={'badge ' + tone}>{label}</span>
            <div className="what selectable">
              {s.action === 'forget' || s.action === 'demote'
                ? <><b>{t?.title ?? s.title}</b><span className="old">{t?.body ?? s.body}</span></>
                : <><b>{s.title}</b>{s.body}{s.action === 'update' && t && <div className="old">Was: {t.body}</div>}</>}
              <div className="why">{s.reason}</div>
              {s.tainted && <div className="why warn-line"><Icon name="shield" size={11} />Drawn from outside content (a page, a file, a forwarded message), not your own words. Accepting keeps it marked unconfirmed.</div>}
              {s.check && <div className="why checked"><Icon name="history" size={11} />{s.check}</div>}
              {checking.has(s.id) && <div className="why"><span className="typing" aria-hidden="true"><i /><i /><i /></span>Replaying past corrections…</div>}
            </div>
            <div className="choices">
              {(s.action === 'add' || s.action === 'update') && !s.check && !checking.has(s.id) && (
                <button className="btn btn-sm btn-ghost" onClick={() => check(s.id)} title="Re-answer past corrections with this change and see if it would have helped. Uses your AI account for a few short replies.">Check</button>
              )}
              <button className="btn btn-sm btn-ghost" onClick={() => decide(s.id, false)}>Dismiss</button>
              <button className="btn btn-sm" onClick={() => decide(s.id, true)}>Accept</button>
            </div>
          </div>
        )
      })}
      {err && <p className="error" style={{ padding: '0 14px' }}>{err}</p>}
    </section>
  )
}

type Skill = {
  id: number; bot_id: number; name: string; description: string; body: string; run_id: number | null; status: 'proposed' | 'active'; created_at: number
  kind: 'learned' | 'patch' | 'merge' | 'retire' | 'taught' | 'saved'; reason: string; replaces: number[]; uses: number; last_used: number | null
}

/** A fix as changed lines: what goes (−) and what comes (+), in order. */
/** The lines that change, in order (a longest-common-subsequence diff), so moving or repeating a step shows too. */
const lineChanges = (before: string, after: string) => {
  const a = before.split('\n'), b = after.split('\n')
  const lcs = a.map(() => new Array<number>(b.length + 1).fill(0)).concat([new Array<number>(b.length + 1).fill(0)])
  for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1])
  const out: { add: boolean; l: string }[] = []
  let i = 0, j = 0
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) { i++; j++ }
    else if (j < b.length && (i >= a.length || lcs[i][j + 1] >= lcs[i + 1][j])) out.push({ add: true, l: b[j++] })
    else out.push({ add: false, l: a[i++] })
  }
  return out
}

/** Procedures learned from runs that worked. Proposed ones wait for you; a bot sees only the ones you accepted. */
/** Skills: the procedures your bots follow step by step, yours and the library's. */
function SkillsView({ chatId }: { chatId: number }) {
  // Adding from the library shows the skill under Your skills straight away.
  const [added, setAdded] = useState(0)
  return (
    <>
      <div className="page-head"><div><h1 className="display">Skills</h1>
        <p className="lead">Step-by-step procedures your bots follow for a task. Add one from the library, teach one, or let a bot learn one from a task that went well. Use one on purpose by typing <code>/</code> in a chat.</p></div></div>
      <Skills key={added} chatId={chatId} />
      <SkillLibrary chatId={chatId} onAdded={() => setAdded(n => n + 1)} />
    </>
  )
}

type LibrarySkill = { slug: string; category: string; name: string; description: string; body: string; needs: string[]; added: number[]; missing: Record<string, string[]>; schedule: { cron: string; label: string; prompt: string } | null }
/** A tool a skill needs, in words (for "Needs …"). */
const NEED: Record<string, string> = {
  calendar_list: 'a linked calendar', email_search: 'an email account', email_draft: 'an email account', meeting_read: 'a transcribed meeting', meeting_search: 'a transcribed meeting', web_fetch: 'reading web pages',
  routine_create: 'routines', chat_search: 'searching earlier chats', memory_search: 'memory', memory_store: 'memory',
}

/** Mimir's own skills: written for its tools, plain text (no scripts), and nothing is added until you add it. */
function SkillLibrary({ chatId, onAdded }: { chatId: number; onAdded: () => void }) {
  const lib = useLoad(() => invoke<LibrarySkill[]>('skill_library', { chatId }), [chatId])
  const bots = useLoad(() => invoke<Bot[]>('bots'), []).data ?? []
  const [target, setTarget] = useState(0)
  const [open, setOpen] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [scheduled, setScheduled] = useState<string[]>([])
  const [err, setErr] = useState<string | null>(null)
  const botOf = useContext(Bots)
  if (!lib.data) return null
  // A routine belongs to one bot: the one picked, or (for all bots) the first.
  const schedule = (k: LibrarySkill) => {
    if (!k.schedule) return
    setBusy(k.slug)
    invoke('add_routine', { chatId, botId: target || bots[0]?.id || 1, cron: k.schedule.cron, prompt: k.schedule.prompt, quiet: false })
      .then(() => { setErr(null); setScheduled(v => [...v, k.slug]) }, e => setErr(String(e))).finally(() => setBusy(null))
  }
  const cats = [...new Set(lib.data.map(k => k.category))]
  // For all bots: what any of them lacks.
  const missingFor = (k: LibrarySkill) => [...new Set(target === 0 ? Object.values(k.missing).flat() : k.missing[String(target)] ?? [])]
  const add = (k: LibrarySkill) => {
    setBusy(k.slug)
    invoke('skill_add', { chatId, slug: k.slug, botId: target }).then(() => { setErr(null); onAdded(); return lib.reload() }, e => setErr(String(e))).finally(() => setBusy(null))
  }
  return (
    <section className="library" aria-label="Skill library">
      <div className="section-title"><h2>From the library</h2><span className="count">{lib.data.length}</span>
        {bots.length > 1 && <select className="select select-inline" value={target} aria-label="Add skills for" onChange={e => setTarget(Number(e.target.value))}>
          <option value={0}>For all bots</option>{bots.map(b => <option key={b.id} value={b.id}>For {b.name}</option>)}
        </select>}</div>
      <p className="hint">Written for Mimir's tools. Adding one gives it to {target === 0 ? 'all your bots' : botOf(target).name} in this chat; you can change or remove it like any skill.</p>
      {err && <p className="error">{err}</p>}
      {cats.map(c => (
        <div key={c} className="lib-cat">
          <h3>{c}</h3>
          <div className="mem-list">{lib.data!.filter(k => k.category === c).map(k => {
            const has = k.added.includes(0) || k.added.includes(target)
            const missing = missingFor(k)
            return (
              <article className="mem" key={k.slug}>
                <div className="mem-actions lib-add">
                  {has ? <span className="badge live"><Icon name="check" size={11} />Added</span>
                    : <button className="btn btn-sm" disabled={busy === k.slug} onClick={() => add(k)}>{busy === k.slug ? 'Adding…' : 'Add'}</button>}
                </div>
                <div className="mem-title">{k.name}</div>
                <p className="mem-body">{k.description}</p>
                {open === k.slug && <div className="skill-steps selectable"><Markdown text={k.body} /></div>}
                <div className="mem-meta">
                  <button type="button" className="linkish" aria-expanded={open === k.slug} onClick={() => setOpen(open === k.slug ? null : k.slug)}>{open === k.slug ? 'hide steps' : 'see the steps'}</button>
                  {missing.length > 0 && <span className="warn" title="It can be added now; it works once these are there.">needs {[...new Set(missing.map(t => NEED[t] ?? t))].join(', ')}</span>}
                  {/* A habit: once added, run it on a schedule in one click (a routine whose prompt is the skill's command). */}
                  {has && k.schedule && (scheduled.includes(k.slug)
                    ? <span>scheduled {k.schedule.label.toLowerCase()} · change it in Routines</span>
                    : <button type="button" className="linkish" disabled={busy === k.slug} onClick={() => schedule(k)}>schedule it · {k.schedule.label.toLowerCase()}</button>)}
                </div>
              </article>
            )
          })}</div>
        </div>
      ))}
    </section>
  )
}

function Skills({ chatId }: { chatId: number }) {
  const list = useLoad(() => invoke<Skill[]>('skills', { chatId }), [chatId], 15000)
  const removed = useLoad(() => invoke<Skill[]>('removed_skills', { chatId }), [chatId, list.data?.map(k => `${k.id}:${k.status}`).join()])
  const [err, setErr] = useState<string | null>(null)
  const { leaving, leave } = useLeave()
  const botOf = useContext(Bots)
  const who = (id: number) => id === 0 ? 'All bots' : botOf(id).name
  const all = list.data ?? []
  const proposed = all.filter(k => k.status === 'proposed'), active = all.filter(k => k.status === 'active')
  const act = (id: number, p: () => Promise<unknown>) => leave(id, () => p().then(() => { setErr(null); return list.reload() }, e => setErr(String(e))))
  const replaces = (k: Skill) => active.some(a => a.bot_id === k.bot_id && a.name.toLowerCase() === k.name.toLowerCase())
  // Teach a skill from a link or pasted text (a proposal like the rest).
  const bots = useLoad(() => invoke<Bot[]>('bots'), []).data ?? []
  const [teaching, setTeaching] = useState<{ source: string; bot: number; busy: boolean } | null>(null)
  const [rehearsal, setRehearsal] = useState<{ id: number; r: Rehearsed | null } | null>(null)
  const rehearse = (k: Skill) => {
    setRehearsal({ id: k.id, r: null })
    invoke<Rehearsed>('rehearse', { chatId, botId: k.bot_id || bots[0]?.id || 1, skillId: k.id }).then(r => setRehearsal({ id: k.id, r }), e => { setRehearsal(null); setErr(String(e)) })
  }
  const teach = () => {
    if (!teaching?.source.trim()) return
    setTeaching({ ...teaching, busy: true })
    invoke('teach_skill', { chatId, botId: teaching.bot, source: teaching.source.trim() })
      .then(() => { setTeaching(null); setErr(null); list.reload() }, e => { setErr(String(e)); setTeaching(t => t && { ...t, busy: false }) })
  }
  return (
    <>
      <div className="teach">
        {teaching ? <form className="er" onSubmit={e => { e.preventDefault(); teach() }}>
          {bots.length > 1 && <select className="select" style={{ width: 'auto' }} value={teaching.bot} onChange={e => setTeaching({ ...teaching, bot: Number(e.target.value) })} aria-label="Which bot learns it">{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select>}
          <input className="input" autoFocus placeholder="A link, the text (how our expense process works, a checklist…), or a SKILL.md" aria-label="What to learn from" value={teaching.source} onChange={e => setTeaching({ ...teaching, source: e.target.value })} />
          <button className="btn btn-sm btn-ghost" type="button" onClick={() => setTeaching(null)}>Cancel</button>
          <button className="btn btn-sm" disabled={teaching.busy || !teaching.source.trim()}>{teaching.busy ? 'Learning…' : 'Learn'}</button>
        </form> : <button className="btn btn-sm btn-ghost" onClick={() => setTeaching({ source: '', bot: bots[0]?.id ?? 1, busy: false })}><Icon name="bolt" size={14} />Teach a skill</button>}
      </div>
      {proposed.length > 0 && (
        <section className="suggest" aria-label="Suggested skills">
          <div className="suggest-head"><Icon name="bolt" /><b>{proposed.length} suggested {proposed.length === 1 ? 'skill' : 'skills'}</b>
            <small>From tasks that went well, fixes after something went wrong, and upkeep. Nothing changes until you accept.</small></div>
          {proposed.map(k => {
            const current = active.find(a => a.bot_id === k.bot_id && a.name.toLowerCase() === k.name.toLowerCase())
            const merging = active.filter(a => k.replaces.includes(a.id))
            return (
              <div className={'sugg' + (leaving.has(k.id) ? ' leaving' : '')} key={k.id}>
                <span className="badge live">{k.kind === 'saved' ? 'Saved from chat' : k.kind === 'taught' ? (current ? 'Taught again' : 'Taught') : k.kind === 'patch' ? 'Fix' : k.kind === 'merge' ? 'Merge' : k.kind === 'retire' ? 'Retire' : replaces(k) ? 'New version' : 'Skill'}</span>
                <div className="what selectable"><b>{k.name}</b>{k.description}
                  {(k.kind === 'patch' || k.kind === 'taught') && current && lineChanges(current.body, k.body).length > 0
                    ? <div className="skill-steps skill-diff">{lineChanges(current.body, k.body).map((c, i) => <div key={i} className={c.add ? 'add' : 'del'}>{c.add ? '+ ' : '− '}{c.l}</div>)}</div>
                    : k.kind === 'retire' ? null : <div className="skill-steps"><Markdown text={k.body} /></div>}
                  {k.kind === 'merge' && merging.length > 0 && <div className="why">Replaces {merging.map(m => `“${m.name}”`).join(' and ')}</div>}
                  {k.reason && <div className="why">{k.reason}</div>}
                  <div className="why">For {who(k.bot_id)}{k.run_id ? `, learned from run #${k.run_id}` : ''}</div></div>
                <div className="choices">
                  <button className="btn btn-sm btn-ghost" onClick={() => act(k.id, () => invoke('decide_skill', { chatId, id: k.id, accept: false }))}>{k.kind === 'retire' ? 'Keep it' : 'Dismiss'}</button>
                  <button className="btn btn-sm" onClick={() => act(k.id, () => invoke('decide_skill', { chatId, id: k.id, accept: true }))}>{k.kind === 'retire' ? 'Retire' : k.kind === 'patch' ? 'Apply fix' : 'Accept'}</button>
                </div>
              </div>
            )
          })}
        </section>
      )}
      {active.length > 0 && (
        <section>
          <div className="section-title"><h2>Your skills</h2><span className="count">{active.length}</span></div>
          <div className="mem-list">{active.map(k => (
            <article className={'mem' + (leaving.has(k.id) ? ' leaving' : '')} key={k.id}>
              <div className="mem-actions">
                <button className="btn btn-ghost btn-sm" disabled={rehearsal?.id === k.id && !rehearsal.r} title="Try it once without sending or changing anything: see what it would do" onClick={() => rehearse(k)}>Rehearse</button>
                <button className="btn btn-ghost btn-sm" title="Copy as a SKILL.md file, for another agent or to share"
                  onClick={() => copyText(invoke<string>('export_skill', { chatId, id: k.id })).then(() => setErr(null), e => setErr(String(e)))}>Copy as SKILL.md</button>
                <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Remove" aria-label={`Remove ${k.name}`}
                  onClick={() => act(k.id, () => invoke('remove_skill', { chatId, id: k.id }))}><Icon name="trash" size={14} /></button>
              </div>
              <div className="mem-title">{k.name}</div>
              <p className="mem-body">{k.description}</p>
              <div className="skill-steps selectable"><Markdown text={k.body} /></div>
              {rehearsal?.id === k.id && (rehearsal.r ? <RehearsalResult r={rehearsal.r} onClose={() => setRehearsal(null)} /> : <p className="hint">Rehearsing… it runs for real, but anything it would send or change is only described.</p>)}
              <div className="mem-meta">
                {bots.length > 1
                  ? <select className="select select-inline" value={k.bot_id} aria-label={`Which bots use ${k.name}`}
                      onChange={e => invoke('share_skill', { chatId, id: k.id, botId: Number(e.target.value) }).then(() => { setErr(null); list.reload() }, er => setErr(String(er)))}>
                      <option value={0}>All bots</option>{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
                    </select>
                  : <span>{who(k.bot_id)}</span>}
                <span>{ago(k.created_at)}</span>
                <span>{k.uses ? `used ${k.uses}× · last ${ago(k.last_used ?? k.created_at)}` : 'not used yet'}</span></div>
            </article>
          ))}</div>
        </section>
      )}
      {(removed.data?.length ?? 0) > 0 && (
        <details className="retired">
          <summary className="hint">{removed.data!.length} retired or removed {removed.data!.length === 1 ? 'skill' : 'skills'}</summary>
          <div className="mem-list">{removed.data!.map(k => (
            <article className="mem" key={k.id}>
              <div className="mem-actions"><button className="btn btn-ghost btn-sm" onClick={() => invoke('restore_skill', { chatId, id: k.id }).then(() => { setErr(null); list.reload() }, e => setErr(String(e)))}>Bring back</button></div>
              <div className="mem-title">{k.name}</div>
              <p className="mem-body">{k.description}</p>
              <div className="mem-meta"><span>{who(k.bot_id)}</span><span>{k.uses ? `used ${k.uses}×` : 'never used'}</span></div>
            </article>
          ))}</div>
        </details>
      )}
      {err && <p className="error">{err}</p>}
    </>
  )
}

type Week = { start: number; turns: number; corrections: number; stops: number; denials: number; tool_calls: number; tool_errors: number; accepted: number; dismissed: number; repeats: number; forgot: number; undone: number }

/** This week against the three before it, as short phrases; only what there's data for. */
function trends(weeks: Week[]): string[] {
  const now = weeks[weeks.length - 1], before = weeks.slice(0, -1)
  if (!now) return []
  const sum = (ws: Week[], k: keyof Week) => ws.reduce((n, w) => n + w[k], 0)
  const per10 = (ws: Week[], k: keyof Week) => sum(ws, 'turns') ? Math.round(sum(ws, k) * 100 / sum(ws, 'turns')) / 10 : null
  const out: string[] = []
  const was = (v: number | null) => v === null ? '' : ` (was ${v})`
  if (now.turns) out.push(`${per10([now], 'corrections')} corrections per 10 messages${was(per10(before, 'corrections'))}`)
  if (now.repeats) out.push(`${now.repeats} repeated an earlier correction`)
  if (now.forgot) out.push(`${now.forgot} “I told you”`)
  if (now.denials || now.stops) out.push(`${now.denials} denied, ${now.stops} stopped`)
  if (now.tool_calls) out.push(`${Math.round(now.tool_errors * 100 / now.tool_calls)}% of actions failed`)
  if (now.accepted + now.dismissed) out.push(`${now.accepted} of ${now.accepted + now.dismissed} suggestions accepted`)
  if (now.undone) out.push(`${now.undone} learned ${now.undone === 1 ? 'memory' : 'memories'} taken back`)
  return out
}

/** The weekly look back: what was learned without you typing it, since you last checked (each can be undone),
 *  and how this week went compared with the ones before. */
function Learned({ chatId, entries, onChange }: { chatId: number; entries: EntryRow[]; onChange: () => void }) {
  // The clock is read when loading, not while rendering.
  const info = useLoad(() => invoke<{ weeks: Week[]; seen: number }>('learning', { chatId }).then(d => ({ ...d, now: Date.now() / 1000 })), [chatId], 60000)
  const [err, setErr] = useState<string | null>(null)
  const { leaving, leave } = useLeave()
  const botOf = useContext(Bots)
  if (!info.data) return null
  const since = Math.max(info.data.seen, info.data.now - 7 * 86400)
  const items = entries.filter(r => r.status === 'active' && r.origin && r.origin !== 'you' && r.created_at > since)
  if (!items.length) return null
  const undo = (id: number) => leave(id, () => invoke('undo_memory', { chatId, id }).then(() => { setErr(null); onChange() }, e => setErr(String(e))))
  const replaced = new Set(entries.map(e => e.superseded_by))
  const trend = trends(info.data.weeks)
  return (
    <section className="suggest weekly" aria-label="Learned this week">
      <div className="suggest-head"><Icon name="memory" /><b>Learned this week</b><small>Saved without you typing it. Undo anything that's wrong.</small>
        <button className="btn btn-sm btn-ghost" onClick={() => invoke('learned_seen', { chatId }).then(() => info.reload(), e => tell(String(e)))}>All good</button></div>
      {items.map(r => (
        <div className={'sugg' + (leaving.has(r.id) ? ' leaving' : '')} key={r.id}>
          <span className="badge">{r.origin === 'tidy' ? 'Tidy-up' : r.origin === 'import' ? 'Imported' : r.origin === 'know_me' ? 'Email & calendar' : r.bot_id ? botOf(r.bot_id).name : 'A bot'}</span>
          <div className="what selectable"><b>{r.title}</b>{r.body}
            {/* Their own words next to what was saved: a remark saved as firmer than they said it shows. */}
            {r.asked && <div className="why">You said: “{r.asked.length > 160 ? r.asked.slice(0, 160) + '…' : r.asked}”</div>}
            <div className="why">{replaced.has(r.id) ? 'Replaced an earlier version; undo brings it back. ' : ''}{ago(r.created_at)}</div></div>
          <div className="choices"><button className="btn btn-sm btn-ghost" onClick={() => undo(r.id)}>Undo</button></div>
        </div>
      ))}
      {trend.length > 0 && <p className="weekly-trend">This week: {trend.join(' · ')}</p>}
      {err && <p className="error" style={{ padding: '0 14px' }}>{err}</p>}
    </section>
  )
}

/** The chat's file workspace: what the bots wrote there (notes, lists, drafts), to read or delete. */
function Files({ chatId }: { chatId: number }) {
  const list = useLoad(() => invoke<{ path: string; size: number }[]>('files', { chatId }), [chatId], 15000)
  const [open, setOpen] = useState<{ path: string; text: string } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  if (!list.data?.length) return null
  const show = (path: string) => open?.path === path ? setOpen(null) : invoke<string>('file', { chatId, path }).then(text => { setErr(null); setOpen({ path, text }) }, e => setErr(String(e)))
  const remove = (path: string) => ask(`Delete ${path}?`, 'Delete', true).then(ok => { if (ok) void invoke('delete_file', { chatId, path }).then(() => { if (open?.path === path) setOpen(null); return list.reload() }, e => setErr(String(e))) })
  return (
    <section aria-label="Files">
      <div className="section-title"><h2>Files</h2><span className="count">{list.data.length}</span></div>
      <p className="hint">Your bots' workspace in this chat: notes, lists and drafts too long for a memory. A rewritten file keeps its earlier version.</p>
      {err && <p className="error">{err}</p>}
      <div className="mem-list">{list.data.map(f => (
        <article className="mem" key={f.path}>
          <div className="mem-actions">
            <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Delete" aria-label={`Delete ${f.path}`} onClick={() => remove(f.path)}><Icon name="trash" size={14} /></button>
          </div>
          <div className="mem-title"><Icon name="file" size={13} /> {f.path}</div>
          {open?.path === f.path && <div className="skill-steps selectable">{f.path.endsWith('.md') ? <Markdown text={open.text} /> : <pre className="file-text">{open.text}</pre>}</div>}
          <div className="mem-meta">
            <button type="button" className="linkish" aria-expanded={open?.path === f.path} onClick={() => show(f.path)}>{open?.path === f.path ? 'hide' : 'open'}</button>
            <span>{f.size < 1024 ? `${f.size} bytes` : `${Math.round(f.size / 1024)} KB`}</span>
          </div>
        </article>
      ))}</div>
    </section>
  )
}

/** Paste another assistant's memories; the memory bot turns them into suggestions to accept or dismiss. */
function ImportMemory({ chatId, onDone }: { chatId: number; onDone: () => void }) {
  const [text, setText] = useState('')
  const [state, setState] = useState<{ busy?: boolean; err?: string; note?: string }>({})
  const go = () => {
    setState({ busy: true })
    invoke<number>('import_memory', { chatId, text }).then(
      n => n ? onDone() : setState({ note: 'Nothing new to add: your memory already has all of it.' }),
      e => setState({ err: String(e) }))
  }
  return (
    <div className="editor-card import">
      <p className="hint" style={{ margin: 0 }}>Paste what another assistant remembers about you. In ChatGPT: Settings → Personalization → Manage memories; or ask any assistant “List everything you remember about me.”</p>
      <textarea className="textarea" rows={7} aria-label="Memories to import" placeholder={'- Lives in Cape Town\n- Prefers short answers\n- Training for a half marathon in March'} value={text} onChange={e => setText(e.target.value)} autoFocus />
      <div className="er">
        <span className="grow">{state.err ? <span className="error">{state.err}</span> : state.note ? <span className="hint">{state.note}</span> : <span className="hint">You'll see each one as a suggestion first. Nothing is saved until you accept it.</span>}</span>
        {state.busy && <span className="typing" aria-hidden="true"><i /><i /><i /></span>}
        <button className="btn btn-primary" disabled={!text.trim() || state.busy} onClick={go}>{state.busy ? 'Reading…' : 'Suggest memories'}</button>
      </div>
    </div>
  )
}

function MemoryCard({ r, versions, act, chatId, showKind }: { r: EntryRow; versions: EntryRow[]; act: (p: Promise<unknown>) => Promise<unknown>; chatId: number; showKind: boolean }) {
  const [editing, setEditing] = useState(false)
  const [showVersions, setShowVersions] = useState(false)
  const [check, setCheck] = useState<{ busy?: boolean; note?: string }>({})
  const botOf = useContext(Bots)
  // Who wrote it is recorded by Mimir; "source" is what the bot said about where it learned it.
  const who = r.origin === 'you' ? 'added by you' : r.origin === 'tidy' ? 'from a tidy-up' : r.origin === 'import' ? 'imported'
    : r.origin === 'bot' && !r.bot_id ? 'suggested by a bot, approved by you'
    : r.origin === 'bot' ? `learned by ${r.bot_id ? botOf(r.bot_id).name : 'a bot'}${r.source ? ` · “${r.source}”` : ''}` : r.source || 'no source'
  return (
    <article className={'mem ' + r.status + (r.pinned ? ' pinned' : '')}>
      {r.status !== 'active' && (
        <div className="mem-actions">
          <button className="btn btn-ghost btn-danger btn-sm" title="Delete it for good, with its earlier versions" onClick={() => {
            ask(`Erase “${r.title}” for good${versions.length ? `, with ${versions.length} earlier version${versions.length === 1 ? '' : 's'}` : ''}? It can't be undone.`, 'Erase', true).then(ok => { if (ok) act(invoke('erase', { chatId, id: r.id })) })
          }}>Erase</button>
        </div>
      )}
      {r.status === 'active' && !editing && (
        <div className="mem-actions">
          <button className={'btn btn-ghost btn-sm icon-btn' + (r.pinned ? ' on' : '')} aria-pressed={!!r.pinned} title={r.pinned ? 'Unpin' : 'Pin: your bots always keep this in mind'} aria-label={`${r.pinned ? 'Unpin' : 'Pin'} ${r.title}`}
            onClick={() => act(invoke('pin', { chatId, id: r.id, pinned: !r.pinned }))}><Icon name="pin" size={14} /></button>
          <button className={'btn btn-ghost btn-sm icon-btn' + (r.background ? ' on' : '')} aria-pressed={!!r.background}
            title={r.background ? 'Bring back: your bots keep this in mind again' : 'Move to background: out of mind, but found when a message needs it'}
            aria-label={`${r.background ? 'Bring back' : 'Move to background'} ${r.title}`}
            onClick={() => act(invoke('background', { chatId, id: r.id, on: !r.background }))}><Icon name="archive" size={14} /></button>
          <button className="btn btn-ghost btn-sm icon-btn" title="Correct" aria-label={`Correct ${r.title}`} onClick={() => setEditing(true)}><Icon name="pencil" size={14} /></button>
          <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Forget" aria-label={`Forget ${r.title}`} onClick={() => act(invoke('forget', { chatId, id: r.id }))}><Icon name="trash" size={14} /></button>
        </div>
      )}
      <div className="mem-title">{r.pinned && <Icon name="pin" size={12} />}{r.title}</div>
      {editing ? (
        <Editor initial={r} submit="Save correction" onCancel={() => setEditing(false)}
          onSave={(kind, title, body) => act(invoke('remember', { chatId, kind, title, body, supersedes: r.id })).then(() => setEditing(false))} />
      ) : (
        <p className="mem-body">{r.body}</p>
      )}
      <div className="mem-meta">
        {showKind && <span>{KIND_SINGULAR[r.kind]}</span>}
        <span>{who}</span>
        {r.tainted && <span className="warn" title="A bot saved this in a conversation that had read outside content (a web page, a file, a tool’s result). Check it says what you meant; until you confirm it, your bots don't keep it in mind (they only see it, labelled, if they search for it).">after reading outside content</span>}
        {r.tainted && r.status === 'active' && <button type="button" className="linkish" onClick={() => act(invoke('confirm_memory', { chatId, id: r.id }))}>Looks right</button>}
        <span>{ago(r.created_at)}</span>
        {r.valid_until && r.status === 'active' && (
          <span className={r.valid_until * 1000 <= Date.now() ? 'gone' : undefined}
            title="Your bots keep it in mind through this day; after that only search finds it, marked as ended.">
            {r.valid_until * 1000 <= Date.now() ? 'ended ' : 'until '}
            {/* sv-SE formats a local date as YYYY-MM-DD, what a date input takes. */}
            <input type="date" className="inline-date" aria-label={`End date of ${r.title}`} value={new Date(r.valid_until * 1000).toLocaleDateString('sv-SE')}
              onChange={e => e.target.value && act(invoke('set_until', { chatId, id: r.id, until: e.target.value }))} />
            {' '}<button type="button" className="linkish" onClick={() => act(invoke('set_until', { chatId, id: r.id, until: null }))}>no end</button>
          </span>
        )}
        {r.access_count > 0 && <span>recalled {r.access_count}×</span>}
        {r.kind === 'convention' && r.status === 'active' && !check.busy && !check.note && (
          <button type="button" className="linkish" title="Re-answer your past corrections about this with and without it, on your bot's current model (useful after switching models). Uses your AI account for a few short replies."
            onClick={() => { setCheck({ busy: true }); invoke<string>('check_memory', { chatId, id: r.id }).then(note => setCheck({ note }), e => setCheck({ note: String(e) })) }}>Check</button>
        )}
        {(r.missteps ?? 0) >= 2 && r.status === 'active' && <span className="warn" title="Replies that used this memory were corrected. If it's wrong or outdated, correct it; the tidy-up will also look at it.">corrected after use {r.missteps}×</span>}
        {versions.length > 0 && <button type="button" className="lineage linkish" aria-expanded={showVersions} onClick={() => setShowVersions(v => !v)}>
          {showVersions ? 'hide' : versions.length === 1 ? 'earlier version' : `${versions.length} earlier versions`}</button>}
        {r.status === 'superseded' && <span className="lineage">replaced by #{r.superseded_by}</span>}
        {r.status === 'forgotten' && <span className="gone">forgotten</span>}
        <span>#{r.id}</span>
      </div>
      {check.busy && <p className="mem-check"><span className="typing" aria-hidden="true"><i /><i /><i /></span>Replaying past corrections…</p>}
      {check.note && (
        <p className="mem-check">{check.note}
          {/* Fixes nothing the model doesn't already get right: it can leave standing memory. */}
          {check.note.startsWith('Fixes 0 ') && check.note.includes('already fine') && !r.background && <>{' '}
            <button type="button" className="linkish" onClick={() => act(invoke('background', { chatId, id: r.id, on: true }))}>Move to background</button></>}
        </p>
      )}
      {showVersions && (
        <ol className="versions">
          {versions.map(v => <li key={v.id}><span className="selectable">{v.title !== r.title && <b>{v.title}: </b>}{v.body}</span><small>{ago(v.created_at)} · #{v.id}</small></li>)}
        </ol>
      )}
    </article>
  )
}

function Editor({ initial, submit, onSave, onCancel }: {
  initial?: { kind: Kind; title: string; body: string }; submit: string
  onSave: (kind: Kind, title: string, body: string) => void; onCancel?: () => void
}) {
  const [kind, setKind] = useState<Kind>(initial?.kind ?? 'knowledge')
  const [title, setTitle] = useState(initial?.title ?? '')
  const [body, setBody] = useState(initial?.body ?? '')
  const ok = title.trim() && body.trim()
  return (
    <form className="editor" onSubmit={e => { e.preventDefault(); if (ok) onSave(kind, title.trim(), body.trim()) }}>
      <div className="er">
        <select className="select" value={kind} onChange={e => setKind(e.target.value as Kind)} aria-label="Type">
          <option value="convention">Preference</option>
          <option value="knowledge">Fact</option>
          <option value="task">Task</option>
        </select>
        <input className="input" placeholder="Short label, e.g. Dentist" value={title} onChange={e => setTitle(e.target.value)} autoFocus />
      </div>
      <textarea className="textarea" placeholder="The fact itself, e.g. My dentist is Dr. Naledi Lee in Stellenbosch." value={body} onChange={e => setBody(e.target.value)} />
      <div className="er" style={{ justifyContent: 'flex-end' }}>
        {onCancel && <button type="button" className="btn btn-ghost" onClick={onCancel}>Cancel</button>}
        <button className="btn btn-primary" disabled={!ok}>{submit}</button>
      </div>
    </form>
  )
}

// ---------------- routines ----------------

/** One-click quiet checks: read-only, and silent unless something needs attention. */
const CHECKINS = [
  { title: 'Morning task check', cron: '30 8 * * Mon-Fri', icon: 'check', prompt: 'Look at my open tasks in memory. If any are due today or overdue, remind me which and why. Otherwise say nothing.' },
  { title: 'Evening look-ahead', cron: '0 19 * * Sun-Thu', icon: 'moon', prompt: 'Check my memory for anything happening tomorrow (appointments, deadlines, people to contact). If there is something, give me a short heads-up.' },
  { title: 'Watch a web page', cron: '0 */3 * * *', icon: 'globe', prompt: 'Tell me if the page reports a problem, or what changed if it matters.', watch: 'https://example.com/status' },
]

// ---- schedule picker: plain choices in, cron out (anything else stays editable as "Custom") ----
type WhenMode = 'daily' | 'weekdays' | 'weekly' | 'hours' | 'custom'
type When = { mode: WhenMode; time: string; day: string; hours: number; cron: string }
const WEEK = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

function toWhen(cron: string): When {
  const base: When = { mode: 'custom', time: '08:00', day: 'Mon', hours: 3, cron }
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
    : w.mode === 'hours' ? `0 */${w.hours} * * *` : w.cron
}

type RoutineDraft = { prompt: string; botId: number; quiet: boolean; when: When; delivery: Delivery; watch: string }

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
          options={[{ id: 'daily', label: 'Every day' }, { id: 'weekdays', label: 'Weekdays' }, { id: 'weekly', label: 'Weekly' }, { id: 'hours', label: 'Every few hours' }, { id: 'custom', label: 'Custom' }]} />
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
        </div>
        {w.mode === 'custom' && <span className="hint">{readable !== cron ? `Runs: ${readable}` : 'Cron format: minute hour day-of-month month weekday, e.g. 30 8 * * Mon-Fri'}</span>}
      </div>
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
          options={[{ id: 'now', label: 'Right away' }, { id: 'digest', label: 'In my daily digest' }, { id: 'log', label: 'Just log it' }]} />
        <span className="hint">{draft.delivery === 'now' ? 'A message (and a notification) as soon as it runs.' : draft.delivery === 'digest' ? 'Collected into one message a day, at the time set in Settings.' : 'Nothing is sent; see what it found in Activity.'}</span>
      </div>
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

function RoutinesView({ chatId, bots }: { chatId: number; bots: Bot[] }) {
  const rows = useLoad(() => invoke<Routine[]>('routines', { chatId }), [chatId])
  const stats = useLoad(() => invoke<RoutineStats[]>('routine_stats', { chatId }), [chatId], 30000)
  const botOf = useContext(Bots)
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
  const update = (r: Routine, patch: { cron?: string; prompt?: string; quiet?: boolean; paused?: boolean; delivery?: Delivery; watch?: string }) =>
    invoke('update_routine', { chatId, id: r.id, cron: r.cron, prompt: r.prompt, quiet: r.quiet, paused: r.paused, ...patch })
      .then(() => patch.watch !== undefined && patch.watch.trim() !== (r.watch ?? '') ? watch(r.id, patch.watch) : null)
      .then(() => patch.delivery && patch.delivery !== r.delivery ? deliver(r.id, patch.delivery) : null)
      .then(() => { setErr(null); setEditing(null); rows.reload() }, e => setErr(String(e)))
  const start = (cron: string, prompt: string, quiet: boolean, w = '') => { setErr(null); setDraft({ prompt, quiet, botId: bots[0]?.id ?? 1, when: toWhen(cron), delivery: 'now', watch: w }) }
  const add = () => draft && invoke<Routine>('add_routine', { chatId, botId: draft.botId, cron: toCron(draft.when), prompt: draft.prompt, quiet: draft.quiet })
    .then(async r => { if (draft.quiet && draft.watch.trim()) await watch(r.id, draft.watch); return r })
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
              if (editing?.id === r.id) return (
                <RoutineForm key={r.id} inline skills={skillRows.data ?? []} draft={editing.draft} setDraft={d => setEditing({ id: r.id, draft: d })} bots={bots} onCancel={() => setEditing(null)}
                  onSave={() => update(r, { cron: toCron(editing.draft.when), prompt: editing.draft.prompt, quiet: editing.draft.quiet, delivery: editing.draft.delivery, watch: editing.draft.quiet ? editing.draft.watch : '' })} />
              )
              return (
                <div className={'routine' + (r.paused ? ' paused' : '')} key={r.id}>
                  <span className={'ri' + (r.quiet ? ' quiet' : '')}><Icon name={r.quiet ? 'moon' : 'clock'} /></span>
                  <div>
                    <div className="rt">{when(r)}{r.quiet && <span className="badge pending" title="Read-only; speaks up only when something needs your attention">Quiet</span>}{r.paused && <span className="badge">Paused</span>}{r.delivery === 'digest' && <span className="badge" title="Collected into your daily digest">In digest</span>}{r.delivery === 'log' && <span className="badge" title="Not sent; see Activity">Log only</span>}{r.quiet && r.watch && <span className="badge" title="Mimir looks first and only runs the AI when this changed">Watching {r.watch === 'mail' ? 'mail' : (() => { try { return new URL(r.watch).host } catch { return r.watch } })()}</span>}</div>
                    <div className="rp selectable">{r.prompt}</div>
                    <div className="rm"><Avatar id={b.id} name={b.name} size={16} still /><span>{b.name}</span><span>·</span><span>{r.paused ? 'Won’t run until you turn it back on' : `${r.once ? 'Runs' : 'Next run'} ${ahead(r.next_fire)}${r.once ? ', then it’s gone' : ''}`}</span></div>
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
                    {!r.once && <button className="btn btn-ghost btn-sm" onClick={() => setEditing({ id: r.id, draft: { prompt: r.prompt, quiet: r.quiet, botId: r.bot_id, when: toWhen(r.cron), delivery: r.delivery ?? 'now', watch: r.watch ?? '' } })}>Edit</button>}
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

// ---------------- activity ----------------

const RUN_TITLE: Record<Run['trigger'], string> = { message: 'Reply', routine: 'Routine', rehearsal: 'Rehearsal', tidy: 'Memory tidy-up', delegated: 'Task from another bot', import: 'Memory import', know_me: 'Getting to know you', check: 'Checked a suggestion' }
/** "Quiet check · Weekdays at 8:30 AM"; a deleted routine keeps its number. */
const runTitle = (r: Run, routine?: Routine) =>
  r.trigger !== 'routine' ? RUN_TITLE[r.trigger] : routine ? `${routine.quiet ? 'Quiet check' : routine.once ? 'One-off' : 'Routine'} · ${when(routine)}` : `Routine #${r.routine_id}`

function ActivityView({ chatId }: { chatId: number }) {
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

// ---------------- bot page ----------------

const PROVIDERS = [
  { id: 'claude', name: 'Claude', hint: 'Your Claude login (Claude Code)' },
  { id: 'chatgpt', name: 'ChatGPT', hint: 'Your ChatGPT plan, signed in' },
  { id: 'codex', name: 'Codex CLI', hint: 'Your ChatGPT login, through the Codex CLI' },
  { id: 'grok', name: 'Grok', hint: 'Your SuperGrok login (Grok Build)' },
  { id: 'openrouter', name: 'OpenRouter', hint: 'Hundreds of models, one key' },
  { id: 'xai', name: 'xAI API', hint: 'Grok, billed per token' },
  { id: 'local', name: 'Local model', hint: 'Ollama or LM Studio, free' },
  { id: 'openai', name: 'API key', hint: 'OpenAI or any compatible API' },
]
const PROVIDER_NOTE: Record<string, React.ReactNode> = {
  claude: <>Runs <code>claude -p</code> with your own login. Anthropic may count use by other apps as extra usage.</>,
  chatgpt: 'Runs on your ChatGPT Plus or Pro plan, signed in under Settings → AI keys. You set how much Mimir may use each week in ChatGPT’s settings.',
  openrouter: 'Uses your OpenRouter key from Settings. OpenRouter bills per message; the run log shows the cost.',
  local: "Runs on this computer (Ollama by default; set the endpoint for LM Studio, http://localhost:1234/v1). Chats and memories don't leave it.",
  grok: <>Runs <code>grok -p</code> on your SuperGrok or X Premium+ subscription. Sign in under Settings → AI keys.</>,
  xai: 'Uses your xAI key from Settings. xAI bills per token.',
  openai: 'Uses the API key from Settings.',
}
const BLANK: Bot = { id: 0, name: '', persona: '', provider: 'claude', model: '', base_url: '', tools: '', has_telegram_token: false, has_slack: false, has_discord: false }
const NO_SECRETS: Secrets = { telegram_token: null, slack_bot_token: null, slack_app_token: null, discord_token: null }

/** A routine a template starts with. Edit the prompt or switch it off before creating the bot. */
type StarterRoutine = { cron: string; prompt: string; quiet?: boolean; delivery?: Delivery; on: boolean }
/** Starting points, by what the bot is responsible for. Each fills in the role and offers routines. */
const TEMPLATES: { id: string; icon: string; name: string; job: string; persona: string; routines: Omit<StarterRoutine, 'on'>[] }[] = [
  { id: 'brief', icon: 'clock', name: 'Morning briefer', job: 'Tells you what matters each morning',
    persona: "You're my morning briefer. Be brief: five bullets at most, most important first. Link your sources.",
    routines: [{ cron: '30 7 * * Mon-Fri', prompt: "Brief me for today: anything in my memory that's due or coming up, and the top tech news worth knowing." }] },
  { id: 'tasks', icon: 'check', name: 'Follow-up keeper', job: 'Remembers your to-dos and nudges you',
    persona: "You keep track of my tasks and promises. When I mention something I need to do, remember it with its due date. When I say it's done, forget it.",
    routines: [{ cron: '0 17 * * Mon-Fri', prompt: "Look at my open tasks and follow-ups in memory. Tell me only what's due tomorrow or overdue.", quiet: true }] },
  { id: 'research', icon: 'search', name: 'Researcher', job: 'Digs into questions, cites sources',
    persona: "You're my research assistant. Read the sources before answering, separate facts from opinions, and always link what you used. Say when you're unsure.",
    routines: [] },
  { id: 'watch', icon: 'globe', name: 'Page watcher', job: 'Checks pages and tells you what changed',
    persona: 'You watch web pages for me. Compare with what you remembered last time, remember what you saw, and only report real changes.',
    routines: [{ cron: '0 9 * * *', prompt: 'Check https://news.ycombinator.com for posts about AI assistants. Tell me only about ones you haven\'t told me before.', quiet: true, delivery: 'digest' }] },
  { id: 'review', icon: 'history', name: 'Weekly reviewer', job: 'Looks back on your week every Friday',
    persona: "You help me reflect. Be honest and specific, and keep it short.",
    routines: [{ cron: '0 16 * * Fri', prompt: "Review my week from our conversations and memory: what got done, what's still open, and one thing to focus on next week." }] },
]

/** A new bot's starter routines run in the built-in chat (results there and as notifications). */
async function addStarterRoutines(botId: number, routines: Omit<StarterRoutine, 'on'>[]) {
  for (const r of routines.filter(r => r.prompt.trim())) {
    const made = await invoke<Routine>('add_routine', { chatId: LOCAL, botId, cron: r.cron, prompt: r.prompt.trim(), quiet: !!r.quiet })
    if (r.delivery && r.delivery !== 'now') await invoke('set_delivery', { chatId: LOCAL, id: made.id, delivery: r.delivery })
  }
}

function Templates({ picked, pick }: { picked: string; pick: (id: string) => void }) {
  return (
    <div className="provider-cards templates" role="radiogroup" aria-label="Start from">
      {[{ id: '', icon: 'plus', name: 'Blank', job: 'A general assistant you shape yourself' }, ...TEMPLATES].map(t => (
        <button type="button" role="radio" aria-checked={picked === t.id} key={t.id} className={'provider-card' + (picked === t.id ? ' on' : '')} onClick={() => pick(t.id)}>
          <b><Icon name={t.icon} size={13} />{t.name}</b><span>{t.job}</span>
        </button>
      ))}
    </div>
  )
}

function BotPage({ bot, status, canDelete, onSaved, onDeleted, onChat }: {
  bot?: Bot; status?: Status; canDelete: boolean; onSaved: (id: number) => void; onDeleted: () => void; onChat?: () => void
}) {
  // A new bot starts as a random creature you can change; it's saved with the bot, so what you see is what you get.
  const [b, setB] = useState<Bot>(bot ?? { ...BLANK, look: JSON.stringify(randomLook()) })
  const look = parseLook(b.look)
  const [styling, setStyling] = useState(!bot)
  const [sec, setSec] = useState<Secrets>(NO_SECRETS)
  const [msg, setMsg] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [template, setTemplate] = useState('')
  const [routines, setRoutines] = useState<StarterRoutine[]>([])
  const isNew = !bot
  const applyTemplate = (id: string) => {
    const t = TEMPLATES.find(x => x.id === id)
    setTemplate(id)
    setRoutines(t?.routines.map(r => ({ ...r, on: true })) ?? [])
    edit({ name: t?.name ?? '', persona: t?.persona ?? '' })
  }
  const setRoutine = (i: number, patch: Partial<StarterRoutine>) => setRoutines(routines.map((r, j) => j === i ? { ...r, ...patch } : r))
  const addRoutines = (botId: number) => addStarterRoutines(botId, routines.filter(r => r.on))
  const typed = (k: keyof Secrets) => sec[k] ?? ''
  const type = (k: keyof Secrets, v: string) => { setSec({ ...sec, [k]: v || null }); setMsg(null) }
  const edit = (patch: Partial<Bot>) => { setB({ ...b, ...patch }); setMsg(null) }
  const chan = (kind: string) => status?.channels.find(c => c.kind === kind)
  const toolServers = useLoad(() => invoke<{ servers: ToolServer[] }>('tool_servers'), [], 60000)
  const serverTools: [string, string][] = (toolServers.data?.servers ?? []).filter(s => !s.preset).map(s => [`${s.name}__*`, `Use ${s.name}`])
  const ALL = [...Object.keys(TOOL_LABEL), 'browser__*', ...serverTools.map(([t]) => t)]
  const enabled = b.tools.trim() ? b.tools.split(',').map(t => t.trim()) : ALL
  const toggle = (tool: string) => {
    const next = enabled.includes(tool) ? enabled.filter(t => t !== tool) : [...enabled, tool]
    edit({ tools: ALL.every(t => next.includes(t)) ? '' : next.join(',') })
  }
  // Which channels are connected comes from the saved bot (it changes on save), not this form's copy.
  const has = (k: 'has_telegram_token' | 'has_slack' | 'has_discord') => !!bot?.[k]
  const form = (x: Bot) => JSON.stringify({ ...x, has_telegram_token: 0, has_slack: 0, has_discord: 0 })
  const dirty = form(b) !== form(bot ?? { ...BLANK, look: b.look }) || Object.values(sec).some(v => v !== null)
  const save = (secrets: Secrets) =>
    invoke<number>('save_bot', { bot: b, secrets })
      .then(async id => {
        if (isNew) await addRoutines(id)
        setSec(NO_SECRETS); setErr(null); setMsg(isNew ? 'Bot created' : 'Saved — the bot restarted with these settings'); onSaved(id)
      }).catch(e => { setErr(String(e)); setMsg(null) })
  const on = online(status)

  return (
    <form onSubmit={e => { e.preventDefault(); save(sec) }} style={{ '--pig': pigment(b.id || 99, look) } as React.CSSProperties}>
      <div className="page-head">
        <button type="button" className="look-avatar" onClick={() => setStyling(s => !s)} aria-expanded={styling} title="Change how it looks">
          <Avatar id={b.id || 99} name={b.name || '?'} size={56} alive look={look} presence={isNew ? undefined : presenceOf(status)} />
          <span className="look-edit"><Icon name="pencil" size={11} /></span>
        </button>
        <div>
          <h1 className="display">{isNew ? (b.name.trim() || 'New bot') : b.name || bot.name}</h1>
          <p className="lead">{isNew ? 'Give it a name, a job, a look and an AI account.' : status?.error ?? [status?.provider, ...on.map(c => `${CHANNEL[c.kind].name} ${c.handle ?? ''}`)].filter(Boolean).join(' · ')}</p>
        </div>
        {!isNew && onChat && <button type="button" className="btn head-action" onClick={onChat}><Icon name="chat" size={14} />Chat</button>}
      </div>
      {styling && <LookPicker id={b.id || 99} look={look} onChange={l => edit({ look: JSON.stringify(l) })} />}
      {on.length > 0 && status?.pair_code && (
        <div className="pair"><Icon name="key" /><span>To talk to {bot?.name} from a new chat, send it <code>pair {status.pair_code}</code> on {on.map(c => CHANNEL[c.kind].name).join(' or ')}.
          {(() => {
            const tg = on.find(c => c.kind === 'telegram' && c.handle)?.handle?.replace(/^@/, '')
            const link = tg && `https://t.me/${tg}?start=${status.pair_code}`
            return link && <> On your phone, just open <a href={link} target="_blank" rel="noreferrer" onClick={openLink}>t.me/{tg}</a> and tap Start.</>
          })()}</span></div>
      )}

      {isNew && <Group title="Start from"><div className="row-block"><Templates picked={template} pick={applyTemplate} /></div></Group>}

      <Group title="Identity">
        <div className="row-block">
          <label className="field-label"><span>Name</span><input className="input" placeholder="e.g. Researcher" value={b.name} onChange={e => edit({ name: e.target.value })}
            // Focused without scrolling: the page opens at its top (the look and the templates come first).
            ref={el => { if (el && isNew && !el.dataset.focused) { el.dataset.focused = '1'; el.focus({ preventScroll: true }) } }} /></label>
          <label className="field-label"><span>Role</span>
            <textarea className="textarea" placeholder="e.g. You're my research assistant. Keep answers short and always link your sources." value={b.persona} onChange={e => edit({ persona: e.target.value })} />
            <span className="hint">Instructions it follows on top of the basics. Leave empty for a general assistant.</span></label>
        </div>
      </Group>

      {isNew && routines.length > 0 && (
        <Group title="Routines it starts with" note="What they find shows up in the built-in chat. Change or pause them any time under Routines.">
          {routines.map((r, i) => (
            <div key={i} className="row starter-routine">
              <div className="row-main">
                <div className="row-title">{schedule(r.cron)}{r.quiet && <span className="badge">Quiet</span>}{r.delivery === 'digest' && <span className="badge">In the digest</span>}</div>
                <textarea className="textarea" aria-label="What it does" rows={2} value={r.prompt} disabled={!r.on} onChange={e => setRoutine(i, { prompt: e.target.value })} />
              </div>
              <div className="row-ctl"><Switch label={`Start with: ${schedule(r.cron)}`} checked={r.on} onChange={on => setRoutine(i, { on })} /></div>
            </div>
          ))}
        </Group>
      )}

      <Group title="AI account" note={PROVIDER_NOTE[b.provider]}>
        <div className="row-block">
          <div className="provider-cards" role="radiogroup" aria-label="AI account">
            {PROVIDERS.map(p => (
              <button type="button" role="radio" aria-checked={b.provider === p.id} key={p.id} className={'provider-card' + (b.provider === p.id ? ' on' : '')} onClick={() => b.provider !== p.id && edit({ provider: p.id, model: '', base_url: '' })}>
                <b>{p.name}</b><span>{p.hint}</span>
              </button>
            ))}
          </div>
          <div className="editor" style={{ margin: 0 }}>
            <div className="er" style={{ alignItems: 'flex-start' }}>
              <label className="field-label" style={{ flex: 1 }}><span>Model</span>
                <ModelInput provider={b.provider} baseUrl={b.base_url} value={b.model} onChange={model => edit({ model })} /></label>
              {(b.provider === 'openai' || b.provider === 'local') && (
                <label className="field-label" style={{ flex: 1.2 }}><span>Endpoint</span>
                  <input className="input" placeholder={b.provider === 'local' ? 'http://localhost:11434/v1' : 'https://api.openai.com/v1'} value={b.base_url} onChange={e => edit({ base_url: e.target.value })} /></label>
              )}
            </div>
            <AccountTest key={`${b.provider}|${b.model}|${b.base_url}`} provider={b.provider} model={b.model} baseUrl={b.base_url} />
          </div>
        </div>
      </Group>

      <Group title="Messaging apps" note={`Each bot is its own Telegram, Slack or Discord bot. Tokens are stored in ${SECRETS_AT}.`}>
        <ChannelRow icon="telegram" title="Telegram" st={chan('telegram')} has={has('has_telegram_token')} onRemove={() => save({ ...NO_SECRETS, telegram_token: '' })}
          hint="Message @BotFather, send /newbot, and paste the token it gives you.">
          <input className="input" type="password" aria-label="Telegram bot token" placeholder="123456:ABC…" value={typed('telegram_token')} onChange={e => type('telegram_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
        <ChannelRow icon="slack" title="Slack" st={chan('slack')} has={has('has_slack')} onRemove={() => save({ ...NO_SECRETS, slack_bot_token: '', slack_app_token: '' })}
          hint="Create a Slack app with Socket Mode and Interactivity on; scopes chat:write, im:history, app_mentions:read, files:read, channels:history, groups:history; events message.im and app_mention (add message.channels and message.groups so it follows a thread without another @-mention).">
          <input className="input" type="password" aria-label="Slack bot token" placeholder="Bot token (xoxb-…)" value={typed('slack_bot_token')} onChange={e => type('slack_bot_token', e.target.value)} autoComplete="off" />
          <input className="input" type="password" aria-label="Slack app-level token" placeholder="App-level token (xapp-…)" value={typed('slack_app_token')} onChange={e => type('slack_app_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
        <ChannelRow icon="discord" title="Discord" st={chan('discord')} has={has('has_discord')} onRemove={() => save({ ...NO_SECRETS, discord_token: '' })}
          hint="In the Discord Developer Portal, create a bot, turn on Message Content Intent, and invite it with Send Messages.">
          <input className="input" type="password" aria-label="Discord bot token" placeholder="Bot token" value={typed('discord_token')} onChange={e => type('discord_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
      </Group>

      <Group title="What it can do" note="Permissions in Settings still decide which of these ask you first.">
        <div className="tool-grid">
          {[...Object.entries(TOOL_LABEL), ['browser__*', 'Use the web browser'], ...serverTools].map(([tool, label]) => (
            <Row key={tool} title={label}><Switch label={label} checked={enabled.includes(tool)} onChange={() => toggle(tool)} /></Row>
          ))}
        </div>
      </Group>

      <div className="savebar">
        {!isNew && canDelete && <button type="button" className="btn btn-ghost btn-danger" onClick={() => {
          ask(`Delete ${bot.name}? Its routines, conversations and run log go with it. Memories stay: they belong to you.`, 'Delete bot', true).then(ok => { if (ok) invoke('delete_bot', { id: bot.id }).then(onDeleted, e => setErr(String(e))) })
        }}><Icon name="trash" size={14} />Delete bot</button>}
        <span className="grow">{err ? <span className="error">{err}</span> : msg ? <span className="ok-note">{msg}</span> : dirty ? <span className="hint">Unsaved changes</span> : null}</span>
        <button className="btn btn-primary" disabled={!b.name.trim() || !dirty}>{isNew ? 'Create bot' : 'Save'}</button>
      </div>
    </form>
  )
}

/** A real round trip with the account + model as currently typed, before you save. */
function AccountTest({ provider, model, baseUrl }: { provider: string; model: string; baseUrl: string }) {
  const [st, setSt] = useState<{ state: 'idle' | 'running' | 'ok' | 'fail'; text?: string }>({ state: 'idle' })
  const run = () => {
    setSt({ state: 'running' })
    invoke<{ seconds: number; provider: string }>('test_account', { provider, model: model.trim() || null, baseUrl: baseUrl.trim() || null })
      .then(r => setSt({ state: 'ok', text: `Works — ${r.provider} replied in ${r.seconds}s` }), e => setSt({ state: 'fail', text: String(e) }))
  }
  return (
    <div className="account-test">
      <button type="button" className="btn btn-sm" disabled={st.state === 'running'} onClick={run}>
        <Icon name="bolt" size={13} />{st.state === 'running' ? 'Testing…' : 'Test'}
      </button>
      {st.state === 'running' && <span className="typing" aria-hidden="true"><i /><i /><i /></span>}
      {st.text && <span className={st.state === 'ok' ? 'ok-note' : 'error'} style={{ margin: 0 }}>{st.text}</span>}
    </div>
  )
}

function ChannelRow({ icon, title, st, has, hint, onRemove, children }: {
  icon: string; title: string; st?: ChannelStatus; has: boolean; hint: string; onRemove: () => void; children: React.ReactNode
}) {
  const [open, setOpen] = useState(false)
  const [label, tone] = st?.connected ? [`Connected · ${st.handle ?? ''}`, 'live'] : st?.error ? ['Not connected', 'danger'] : has ? ['Connecting…', 'pending'] : ['Not set up', '']
  return (
    <>
      <div className="row chan-row">
        <span className="row-icon"><Icon name={icon} size={15} /></span>
        <div className="row-main"><div className="row-title">{title}</div>{st?.error && <div className="row-desc" style={{ color: 'var(--danger)' }}>{st.error}</div>}</div>
        <div className="row-ctl">
          <span className={'badge ' + tone}>{label}</span>
          {has && <button type="button" className="btn btn-sm btn-ghost btn-danger" onClick={onRemove}>Disconnect</button>}
          <button type="button" className="btn btn-sm" onClick={() => setOpen(o => !o)}>{open ? 'Done' : has ? 'Change' : 'Set up'}</button>
        </div>
      </div>
      {open && <div className="row-block">{children}<span className="hint">{hint}</span></div>}
    </>
  )
}

// ---------------- settings ----------------

/** Words a setting goes by besides its own text, so a search finds it ("token" → AI keys, "2fa"…). */
const SETTING_WORDS: Record<string, string> = {
  'AI keys': 'api key token openai openrouter xai grok anthropic model provider chatgpt sign in plan',
  'Email': 'mail imap smtp gmail icloud fastmail inbox app password send draft',
  'Calendar': 'caldav icloud fastmail nextcloud events schedule',
  'Web search': 'brave searxng google internet search engine',
  'Daily digest': 'quiet hours notifications summary weekly review do not disturb',
  'Limits': 'cap budget tokens cost usage limit plan subscription quota five hour weekly cheaper model haiku',
  'Permissions': 'allow ask deny approve rules tools safety always',
  'Browser': 'playwright chromium saved logins passwords vault',
  'Tool servers': 'mcp plugins integrations connectors',
  'Your data': 'export backup download delete privacy',
}

/** Shows only the settings that match, live as you type: whole groups whose title or note matches, else the rows that do. */
function useSettingsFilter(q: string) {
  const ref = useRef<HTMLDivElement>(null)
  const [none, setNone] = useState(false)
  useEffect(() => {
    const root = ref.current
    if (!root) return
    const words = q.toLowerCase().split(/\s+/).filter(Boolean)
    const text = (el: Element) => [el.textContent, ...[...el.querySelectorAll('input, textarea, select, [aria-label]')].map(i => `${i.getAttribute('placeholder') ?? ''} ${i.getAttribute('aria-label') ?? ''}`)].join(' ').toLowerCase()
    const hits = (t: string) => words.every(w => t.includes(w))
    const apply = () => {
      let shown = 0
      for (const g of root.querySelectorAll<HTMLElement>('section.group-wrap')) {
        const head = g.querySelector('.group-head')?.textContent ?? ''
        const own = `${head} ${g.querySelector('.group-note')?.textContent ?? ''} ${SETTING_WORDS[head.trim()] ?? ''}`.toLowerCase()
        const rows = [...g.querySelectorAll<HTMLElement>('.group > *')]
        const whole = !words.length || hits(own)
        let any = whole
        for (const r of rows) {
          const show = whole || hits(text(r))
          r.hidden = !show
          any ||= show
        }
        g.hidden = !any
        if (any) shown++
      }
      setNone(words.length > 0 && shown === 0)
    }
    apply()
    // Settings load as they go (each group fetches its own state): filter what turns up, too.
    const mo = new MutationObserver(() => { mo.disconnect(); apply(); mo.observe(root, { childList: true, subtree: true }) })
    mo.observe(root, { childList: true, subtree: true })
    return () => mo.disconnect()
  }, [q])
  return { ref, none }
}

function SettingsView({ onComputer, find = '' }: { onComputer: () => void; find?: string }) {
  // Adding a tool server (or the browser) adds tools: show their permissions straight away.
  const [tools, setTools] = useState(0)
  const toolsChanged = () => setTools(n => n + 1)
  const [q, setQ] = useState(find)
  const box = useRef<HTMLInputElement>(null)
  const { ref, none } = useSettingsFilter(q)
  useEffect(() => {
    // ⌘F / Ctrl+F (handled with the app's other shortcuts) or / finds a setting; Escape clears the search.
    const key = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && e.target.closest('input, textarea, select, [contenteditable]')
      if (e.key === '/' && !typing) { e.preventDefault(); box.current?.focus(); box.current?.select() }
      else if (e.key === 'Escape' && e.target === box.current) setQ('')
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [])
  return (
    <>
      <div className="page-head"><div><h1 className="display">Settings</h1><p className="lead">How your bots run, what they may do, and where they live.</p></div></div>
      <label className="search settings-search"><Icon name="search" size={15} />
        <input ref={box} className="input" type="search" placeholder={`Search settings  ${shortcut('F')}`} aria-label="Search settings" value={q} onChange={e => setQ(e.target.value)} autoFocus={!('ontouchstart' in window)} />
      </label>
      {none && <Empty icon="search" title={`Nothing in Settings matches “${q.trim()}”`}>A bot's name, role, AI account, tools and its Telegram, Slack or Discord connection are on its own page: right-click the bot in the sidebar.</Empty>}
      {!none && /telegram|slack|discord|role|persona|pair|bot token|model/i.test(q) && <p className="hint" style={{ margin: '-12px 0 18px' }}>Each bot's Telegram, Slack and Discord connection, role and AI account are on its own page: right-click the bot in the sidebar.</p>}
      <div ref={ref}>
      {inDesktop && !remoteServer() && <AppSettings />}
      {inDesktop && <UpdateSettings />}
      <DigestSettings />
      <LimitsSettings />
      <ApiKeySettings />
      <WebSearchSettings />
      <EmailSettings />
      <CalendarSettings />
      <KnowMeSettings />
      <ComputerSettings onChange={() => { onComputer(); toolsChanged() }} />
      <BrowserSettings onChange={toolsChanged} />
      <ToolServerSettings onChange={toolsChanged} />
      <Permissions key={tools} />
      <DataSettings />
      <ServerSettings />
      </div>
    </>
  )
}

/** Take everything with you, as one file. */
function DataSettings() {
  const [msg, setMsg] = useState<string | null>(null)
  const exportAll = () => invoke<unknown>('export_all')
    .then(data => saveDownload(`mimir-export-${new Date().toISOString().slice(0, 10)}.json`, JSON.stringify(data, null, 2)))
    .then(where => setMsg(`Saved to ${where}.`), e => setMsg(String(e)))
  return (
    <Group title="Your data" note="Memories, conversations, meetings, skills and routines are yours. Delete a chat's history from its Activity tab; erase a memory from Memory.">
      <Row icon={<Icon name="archive" size={15} />} title="Export everything" desc={msg ?? 'One JSON file with everything Mimir keeps about you (not your tokens or keys).'}>
        <button type="button" className="btn btn-sm" onClick={exportAll}>Export</button>
      </Row>
    </Group>
  )
}

/** The bots' own computer (self-hosted Linux server): a screen with a browser they can see and use. */
function ComputerSettings({ onChange }: { onChange: () => void }) {
  const st = useLoad(() => invoke<ComputerStatus>('computer'), [], 15000)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  if (!st.data) return null
  const toggle = (on: boolean) => {
    setBusy(true); setErr(null)
    invoke('computer_enable', { on }).then(() => { st.reload(); onChange() }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  return (
    <Group title="Computer" note={err ? <span className="error">{err}</span> : st.data.available
      ? 'A Linux desktop with Chromium, inside this server. Bots look at it in screenshots and use the mouse and keyboard, like a person; the browser tools use the same Chromium. Opening a site asks first (or "always on this site"); saved logins ask every time. Watch or take over from Computer in the sidebar.'
      : 'Runs on a self-hosted Linux server: the Mimir Docker image has everything it needs. Not on this computer.'}>
      <Row icon={<Icon name="laptop" size={15} />} title="Give your bots a computer" desc={!st.data.available ? 'Not available here' : st.data.enabled ? (st.data.running ? 'On: the screen is running' : 'On: starting…') : 'Off'}>
        <Switch label="Give your bots a computer" checked={st.data.enabled} disabled={busy || !st.data.available} onChange={toggle} />
      </Row>
    </Group>
  )
}

type MeetingLine = { ts: number; who: string; text: string }
type MeetingStatus = { recording: boolean; started_at: number; lines: MeetingLine[]; sources: string[]; pending: number; error?: string | null; warning?: string | null; mic_warning?: string | null; preparing?: string | null; meeting_id?: number | null }
type MeetingRow = { id: number; title: string; started_at: number; ended_at: number; words: number; notes: string; speakers?: Record<string, string>; summary?: string }
/** "You", a name the person gave, "Speaker 2" once voices are told apart, else "Others". */
const speaker = (who: string, names: Record<string, string> = {}) =>
  who === 'you' ? 'You' : who.startsWith('others:') ? names[who] ?? `Speaker ${who.slice(7)}` : 'Others'
const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

/** A transcript as plain text, for copying and for handing to a bot. */
const transcriptText = (lines: MeetingLine[], start: number, names: Record<string, string> = {}) =>
  lines.map(l => `[${mmss(Math.max(0, l.ts - start))}] ${l.who === 'you' ? 'Me' : speaker(l.who, names)}: ${l.text}`).join('\n')

/** Meetings and calls, transcribed on this computer: your microphone is "you", the computer's sound is the others. */
function MeetingsView({ bots, onSent }: { bots: Bot[]; onSent: (bot: number) => void }) {
  const [st, setSt] = useState<MeetingStatus | null>(null)
  const [engine, setEngine] = useState<{ engine: string | null; model: string; model_ready: boolean } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [open, setOpen] = useState<number | null>(null)
  const [offer, setOffer] = useState<boolean | null>(null)
  // The calendar's private iCal link: '' (none), 'linked', or the text being pasted.
  const [cal, setCal] = useState('')
  const [calNote, setCalNote] = useState<string | null>(null)
  const linkCalendar = (url: string) => localInvoke<{ linked: boolean; this_week?: number }>('meeting', { action: 'calendar', url })
    .then(r => { setCal(r.linked ? 'linked' : ''); setCalNote(r.linked ? `${r.this_week} ${r.this_week === 1 ? 'meeting' : 'meetings'} this week.` : null) }, e => setCalNote(String(e)))
  const [mics, setMics] = useState<{ mics: string[]; chosen: string | null } | null>(null)
  const [notes, setNotes] = useState('')
  const [localBots, setLocalBots] = useState<Bot[]>([])
  const setting = (key: string, value: string) => localInvoke('meeting', { action: 'set', [key]: value }).catch(e => setErr(String(e)))
  // Saved on this computer as they're transcribed (even when the app shows a server).
  const list = useLoad(() => localInvoke<MeetingRow[]>('meeting', { action: 'list' }), [], 15000)
  const [, tick] = useState(0)
  useEffect(() => {
    localInvoke<typeof engine>('meeting', { action: 'engine' }).then(setEngine, () => {})
    localInvoke<boolean>('meeting', { action: 'offer' }).then(setOffer, () => {})
    localInvoke<{ linked: boolean }>('meeting', { action: 'calendar' }).then(c => setCal(c.linked ? 'linked' : ''), () => {})
    localInvoke<{ mics: string[]; chosen: string | null }>('meeting', { action: 'mics' }).then(setMics, () => {})
    localInvoke<{ meeting_notes: string | null }>('meeting', { action: 'set' }).then(s => setNotes(s.meeting_notes ?? ''), () => {})
    // The bots on this computer (they write notes when a meeting ends, here, even when the app shows a server).
    localInvoke<Bot[]>('bots').then(setLocalBots, () => {})
    let live = true
    let first = true
    const poll = () => localInvoke<MeetingStatus>('meeting', { action: 'status' }).then(s => {
      if (!live) return
      // Opened from the tray's "Stop transcribing": show the meeting that just ended.
      if (first && !s.recording && s.meeting_id) setOpen(s.meeting_id)
      first = false; setSt(s); tick(n => n + 1)
    }, () => {}).finally(() => { if (live) setTimeout(poll, 1200) })
    poll()
    return () => { live = false }
  }, [])
  const start = () => {
    setBusy(true); setErr(null)
    localInvoke<MeetingStatus>('meeting', { action: 'start' }).then(setSt, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const stop = () => {
    setBusy(true)
    localInvoke<MeetingStatus>('meeting', { action: 'stop' }).then(s => { setSt(s); list.reload(); setOpen(s.meeting_id ?? null) }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const recording = !!st?.recording
  const saved = (list.data ?? []).filter(m => !recording || m.id !== st!.meeting_id)
  // Search every transcript ("deck", a name, a topic): matching lines, each opening its meeting.
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<(MeetingLine & { id: number; title: string; started_at: number })[]>([])
  useEffect(() => {
    if (!query.trim()) { setHits([]); return }
    const t = setTimeout(() => localInvoke<typeof hits>('meeting', { action: 'search', query: query.trim() }).then(setHits, () => {}), 200)
    return () => clearTimeout(t)
  }, [query])
  return (
    <>
      <div className="page-head"><div><h1 className="display">Meetings</h1>
        <p className="lead">Transcribe meetings and phone calls on this computer: Teams, Zoom, Meet, Webex, Slack, WhatsApp, Teams or Zoom Phone, RingCentral and other softphones. Then hand it to a bot for a summary and follow-ups.</p></div></div>
      {err && <p className="error">{err}</p>}
      {engine && !engine.engine && (
        <div className="howto"><Icon name="alert" /><span>To transcribe, install whisper.cpp ({platform === 'mac' ? <code>brew install whisper-cpp</code> : <>put <code>whisper-cli</code> from github.com/ggml-org/whisper.cpp on your PATH</>}; it runs on this computer, private and free), or add an OpenAI-compatible key in Settings.</span></div>
      )}
      <div className={'meeting-card' + (recording ? ' live' : '')}>
        {recording ? <>
          <div className="mc-head"><span className="rec-dot" /><b>Transcribing</b><span className="mono">{mmss(Date.now() / 1000 - st!.started_at)}</span>
            <span className="hint">{st!.preparing ?? st!.sources.join(' + ') + (st!.pending ? ' · writing…' : '')}</span>
            <button className="btn btn-primary" disabled={busy} onClick={stop}>{busy ? 'Finishing…' : 'Stop'}</button></div>
          <p className="hint" style={{ margin: 0 }}>New lines appear a few seconds after each pause. Let the others know you're transcribing.</p>
        </> : <>
          <div className="mc-head"><b>Start when the meeting starts</b>
            <button className="btn btn-primary" disabled={busy || (engine !== null && !engine.engine)} onClick={start}><Icon name="mic" size={14} />{busy ? 'Starting…' : 'Start transcribing'}</button></div>
          <p className="hint" style={{ margin: 0 }}>Records your microphone (you) and this computer's sound (everyone else), and transcribes {engine?.engine === 'local' ? `here with whisper.cpp (${engine.model} model${engine.model_ready ? '' : ', downloads once'})` : 'with your OpenAI-compatible key'}. Saved as it goes. Headphones keep the two sides apart. A call on your mobile? Take it on the computer ({platform === 'win' ? 'Phone Link' : platform === 'mac' ? 'iPhone calls on your Mac' : 'or put it on speaker, then it all reads as You'}) and it's transcribed the same way. Recording a call usually needs the others' consent.</p>
          <div className="mc-opts">
            {(mics?.mics.length ?? 0) > 1 && <label>Your microphone
              <select className="select" value={mics!.chosen ?? ''} onChange={e => { setMics({ ...mics!, chosen: e.target.value }); setting('meeting_mic', e.target.value) }}>
                <option value="">System default</option>{mics!.mics.map(m => <option key={m}>{m}</option>)}
              </select></label>}
            {localBots.length > 0 && <label>When it ends
              <select className="select" value={notes} onChange={e => { setNotes(e.target.value); setting('meeting_notes', e.target.value) }}>
                <option value="">Just save the transcript</option>{localBots.map(b => <option key={b.id} value={String(b.id)}>{b.name} writes the notes</option>)}
              </select></label>}
          </div>
          <div className="mc-cal">
            <span>Calendar <span className="hint">· names recordings after the meeting, knows who's invited, and offers to transcribe when one starts</span></span>
            {cal === 'linked'
              ? <div className="er"><span className="hint grow">Linked (the link is kept in {keystore})</span><button className="btn btn-ghost btn-sm" onClick={() => linkCalendar('')}>Unlink</button></div>
              : <form className="er" onSubmit={e => { e.preventDefault(); if (cal.trim()) linkCalendar(cal.trim()) }}>
                  <input className="input" type="url" placeholder="Your calendar's secret iCal link (Google: Settings → your calendar → Secret address in iCal format)" aria-label="Calendar iCal link" value={cal} onChange={e => setCal(e.target.value)} />
                  <button className="btn btn-sm" disabled={!cal.trim()}>Link</button>
                </form>}
            {calNote && <span className="hint">{calNote}</span>}
          </div>
          <label className="mc-offer"><Switch label="Offer to transcribe when a call starts" checked={!!offer} disabled={offer === null}
            onChange={v => localInvoke<boolean>('meeting', { action: 'offer', on: v }).then(setOffer, e => setErr(String(e)))} />
            <span>Offer to transcribe when a call starts <span className="hint">(a notification once another app has used the microphone for 20 seconds)</span></span></label>
        </>}
        {!recording && st?.preparing && <p className="hint" style={{ margin: 0 }}>{st.preparing}</p>}
        {st?.error && <p className="error" style={{ margin: 0 }}>{st.error}</p>}
        {recording && [st!.mic_warning, st!.warning].filter(Boolean).map(w => <div key={w!} className="howto"><Icon name="alert" /><span>{w}</span></div>)}
      </div>
      {recording && st!.meeting_id && <LiveNotes id={st!.meeting_id} />}
      {recording && st!.lines.length > 0 && <Transcript lines={st!.lines} start={st!.started_at} />}
      {saved.length > 0 && <>
        <div className="section-title"><h2>Saved</h2><span className="count">{saved.length}</span>
          <input className="input meeting-search" type="search" placeholder="Search what was said" aria-label="Search meetings" value={query} onChange={e => setQuery(e.target.value)} /></div>
        {query.trim() && <div className="transcript">{hits.length ? hits.map((h, i) => (
          <button key={i} type="button" className="tl hit" onClick={() => { setOpen(h.id); setQuery('') }}>
            <span className="mono">{mmss(Math.max(0, h.ts - h.started_at))}</span><b>{speaker(h.who, list.data?.find(m => m.id === h.id)?.speakers)}</b>
            <span>{h.text} <span className="hint">· {meetingTitle(h)}</span></span>
          </button>
        )) : <span className="hint">Nothing said matches.</span>}</div>}
        <div className="mem-list">{saved.map(m => (
          <SavedMeeting key={m.id} m={m} open={open === m.id} toggle={() => setOpen(open === m.id ? null : m.id)} bots={bots} onSent={onSent} onChange={list.reload} />
        ))}</div>
      </>}
    </>
  )
}

/** A notepad during the call: a few words on what matters, saved as you type; the notes bot builds around them. */
function LiveNotes({ id }: { id: number }) {
  const [text, setText] = useState('')
  const [saved, setSaved] = useState(true)
  // What's typed but not yet saved, for the save on the way out: Stop hides this box at once, often inside the 700 ms.
  const unsaved = useRef<string | null>(null)
  useEffect(() => {
    if (saved) return
    const t = setTimeout(() => localInvoke('meeting', { action: 'notes', id, notes: text }).then(() => { unsaved.current = null; setSaved(true) }, () => {}), 700)
    return () => clearTimeout(t)
  }, [text, saved, id])
  useEffect(() => () => { if (unsaved.current !== null) localInvoke('meeting', { action: 'notes', id, notes: unsaved.current }).catch(() => {}) }, [id])
  return (
    <label className="live-notes">
      <span>Your notes <span className="hint">{text ? (saved ? '· saved' : '· saving…') : '· a few words on what matters; the notes build around them'}</span></span>
      <textarea className="input meeting-notes" rows={4} value={text} placeholder={'- budget needs sign-off\n- I own the landing page'} onChange={e => { setText(e.target.value); unsaved.current = e.target.value; setSaved(false) }} />
    </label>
  )
}

function Transcript({ lines, start, names, onName }: { lines: MeetingLine[]; start: number; names?: Record<string, string>; onName?: (key: string) => void }) {
  return (
    <div className="transcript selectable">{lines.map((l, i) => (
      <div key={i} className={'tl ' + (l.who === 'you' ? 'you' : 'others')}><span className="mono">{mmss(Math.max(0, l.ts - start))}</span>
        {/* the name once per turn, not on every sentence; a told-apart speaker can be named */}
        {lines[i - 1]?.who === l.who ? <b /> : onName && l.who.startsWith('others:')
          ? <button type="button" className="tl-name" title="Name this speaker" onClick={() => onName(l.who)}>{speaker(l.who, names)}</button>
          : <b>{speaker(l.who, names)}</b>}
        <span>{l.text}</span></div>
    ))}</div>
  )
}

const meetingTitle = (m: Pick<MeetingRow, 'title' | 'started_at'>) => m.title || `Meeting, ${new Date(m.started_at * 1000).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}`

function SavedMeeting({ m, open, toggle, bots, onSent, onChange }: { m: MeetingRow; open: boolean; toggle: () => void; bots: Bot[]; onSent: (bot: number) => void; onChange: () => void }) {
  const detail = useLoad(() => open ? localInvoke<{ lines: MeetingLine[] }>('meeting', { action: 'get', id: m.id }) : Promise.resolve(null), [open, m.id], 60000)
  const [title, setTitle] = useState(m.title)
  const [jotted, setJotted] = useState(m.notes ?? '')
  const rename = () => { if (title.trim() !== m.title) localInvoke('meeting', { action: 'rename', id: m.id, title: title.trim() }).then(onChange, e => setNote(String(e))) }
  const [bot, setBot] = useState(bots[0]?.id ?? 1)
  const [note, setNote] = useState<string | null>(null)
  const lines = detail.data?.lines ?? []
  const send = () => {
    // What you wrote is yours; the transcript (and the invite's title) are other people's words, so they go as an
    // attached file: outside content, which the bot reads but doesn't take orders from.
    const said = `Summarise my meeting from ${new Date(m.started_at * 1000).toLocaleString()} (${mmss(m.ended_at - m.started_at)} long), attached: `
      + 'decisions, action items with who owns them, and anything I promised.'
      + (jotted.trim() ? `\n\nMy notes during the call (what mattered to me; build around them):\n${jotted.trim()}` : '')
    const transcript = `Meeting: ${title.trim() || meetingTitle(m)}\n\n${transcriptText(lines, m.started_at, m.speakers)}`
    // Base64 in pieces: one call over an hour's transcript would overflow the stack.
    const bytes = new TextEncoder().encode(transcript)
    let bin = ''
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
    const data = btoa(bin)
    invoke<string>('read_document', { botId: bot, chatId: LOCAL, name: 'meeting transcript.txt', data, text: said })
      .then(text => invoke<string>('send_local', { botId: bot, text, images: [] }))
      // The bot's reply is kept on the meeting too (the chat shows it as it's written).
      .then(reply => localInvoke('meeting', { action: 'summary', id: m.id, summary: reply }).then(onChange))
      .catch(e => setNote(String(e)))
    // To the chat with the bot that's writing the notes, so its reply is what you see.
    onSent(bot)
  }
  return (
    <article className="mem">
      <button type="button" className="meeting-row" onClick={toggle} aria-expanded={open}>
        <span className="mem-title">{meetingTitle(m)}</span>
        <span className="hint">{new Date(m.started_at * 1000).toLocaleString()} · {mmss(m.ended_at - m.started_at)} · {m.words} words</span>
      </button>
      {open && <>
        <input className="input" style={{ marginTop: 10 }} placeholder="Name it, e.g. Weekly standup" aria-label="Meeting title" value={title}
          onChange={e => setTitle(e.target.value)} onBlur={rename} onKeyDown={e => e.key === 'Enter' && e.currentTarget.blur()} autoFocus={!m.title} />
        {m.summary && <div className="meeting-summary selectable"><Markdown text={m.summary} /></div>}
        <textarea className="input meeting-notes" placeholder="Your notes (what mattered to you)" aria-label="Your notes" value={jotted} rows={Math.min(8, Math.max(2, jotted.split('\n').length))}
          onChange={e => setJotted(e.target.value)} onBlur={() => { if (jotted !== m.notes) localInvoke('meeting', { action: 'notes', id: m.id, notes: jotted }).then(onChange, e => setNote(String(e))) }} />
        {lines.length ? <Transcript lines={lines} start={m.started_at} names={m.speakers} onName={key => {
          askText(`Who is ${speaker(key, m.speakers)}?`, m.speakers?.[key] ?? '').then(name => {
            if (name !== null) localInvoke('meeting', { action: 'speaker', id: m.id, key, name }).then(onChange, e => setNote(String(e)))
          })
        }} /> : <p className="hint">Nothing was said, or it was too quiet to transcribe.</p>}
        <div className="er" style={{ marginTop: 10 }}>
          {bots.length > 1 && <select className="select" style={{ width: 'auto' }} value={bot} onChange={e => setBot(Number(e.target.value))} aria-label="Which bot">{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select>}
          <button className="btn btn-primary" disabled={!lines.length} onClick={send}>{m.summary ? 'Summarise again with' : 'Summarise with'} {bots.find(b => b.id === bot)?.name ?? 'your bot'}</button>
          <button className="btn btn-ghost" disabled={!lines.length} onClick={() => copyText(transcriptText(lines, m.started_at, m.speakers)).then(() => setNote('Copied'))}>Copy</button>
          <span className="grow" />
          <button className="btn btn-ghost btn-danger" onClick={() => ask(`Delete “${meetingTitle(m)}”?`, 'Delete', true).then(ok => { if (ok) localInvoke('meeting', { action: 'delete', id: m.id }).then(onChange, e => tell(`Couldn't delete it: ${e}`)) })}>Delete</button>
        </div>
        {note && <p className="hint">{note}</p>}
      </>}
    </article>
  )
}

/** Keys from the browser → xdotool names. Printable characters are typed instead. */
const XKEY: Record<string, string> = {
  Enter: 'Return', Backspace: 'BackSpace', Tab: 'Tab', Escape: 'Escape', Delete: 'Delete', ArrowLeft: 'Left', ArrowRight: 'Right',
  ArrowUp: 'Up', ArrowDown: 'Down', Home: 'Home', End: 'End', PageUp: 'Prior', PageDown: 'Next', ' ': 'space',
}

/** Watch the bots' computer live, and take over: click, type, press keys and scroll on it yourself. */
function ComputerView({ bots, chats, chatName }: { bots: Bot[]; chats: Chat[]; chatName: (c: Chat) => string }) {
  type Shot = { image: string; width: number; height: number; using?: string | null; teaching?: { seconds: number; steps: number; limit: number } | null }
  const [shot, setShot] = useState<Shot | null>(null)
  // Teach by showing: who the skill is for, and how drafting went.
  const [forBot, setForBot] = useState(bots[0]?.id ?? 1)
  const [forChat, setForChat] = useState(chats[0]?.chat_id ?? 0)
  const [teach, setTeach] = useState<{ busy?: boolean; done?: string; err?: string }>({})
  const teachDo = (action: string) => {
    setTeach({ busy: action === 'stop' })
    invoke<number | null>('computer_teach', { action, botId: forBot, chatId: forChat }).then(
      id => setTeach(action === 'stop' ? { done: id ? 'Drafted. It’s waiting for you in Memory → Skills of that chat.' : 'A version of that skill is already waiting in Memory → Skills.' } : {}),
      e => setTeach({ err: String(e) }))
  }
  const t = shot?.teaching
  const mmss = (n: number) => `${Math.floor(n / 60)}:${String(n % 60).padStart(2, '0')}`
  const [err, setErr] = useState<string | null>(null)
  const [focus, setFocus] = useState(false)
  const img = useRef<HTMLImageElement>(null)
  useEffect(() => {
    let live = true
    const tick = () => invoke<Shot>('computer_screen')
      .then(s => { if (live) { setShot(s); setErr(null) } }, e => live && setErr(String(e)))
      .finally(() => { if (live) setTimeout(tick, 700) })
    tick()
    return () => { live = false }
  }, [])
  const input = (a: Record<string, unknown>) => invoke('computer_input', a).catch(e => setErr(String(e)))
  // Screen pixels from a point on the (scaled) picture.
  const at = (e: React.MouseEvent | React.WheelEvent) => {
    const r = img.current!.getBoundingClientRect()
    return { x: Math.round((e.clientX - r.left) * (shot!.width / r.width)), y: Math.round((e.clientY - r.top) * (shot!.height / r.height)) }
  }
  const onKey = (e: React.KeyboardEvent) => {
    if (e.metaKey) return
    const mods = [e.ctrlKey && 'ctrl', e.altKey && 'alt'].filter(Boolean) as string[]
    // AltGr characters (@ € { on many layouts) arrive as Ctrl+Alt: they're typed text, not a shortcut.
    if (e.key.length === 1 && (!mods.length || e.getModifierState('AltGraph'))) { e.preventDefault(); input({ kind: 'type', text: e.key }); return }
    const k = XKEY[e.key] ?? (e.key.length === 1 ? e.key.toLowerCase() : null)
    if (!k) return
    e.preventDefault()
    input({ kind: 'key', keys: [...mods, ...(e.shiftKey && e.key.length > 1 ? ['shift'] : []), k].join('+') })
  }
  return (
    <>
      <div className="page-head"><div><h1 className="display">Computer</h1>
        <p className="lead">Your bots' screen, live. Click into it to take over: your clicks, typing and scrolling go to the computer.</p></div></div>
      {err && <p className="error">{err}</p>}
      {shot && (
        <div className="computer-using">
          {shot.using === 'you'
            ? <><span className="dot live" />{shot.teaching ? 'You’re in control while you show. Bots wait.' : 'You’re in control. Bots wait until you give it back (or a minute after your last click).'}
              <button className="btn btn-sm" onClick={() => invoke('computer_release').catch(e => setErr(String(e)))}>Give back</button></>
            : shot.using ? <>{(() => { const u = bots.find(b => b.name === shot.using); return u ? <Avatar id={u.id} name={u.name} size={20} mood="reading" /> : <span className="dot pending" /> })()}{shot.using} is using the computer</>
            : <><span className="dot" />Idle</>}
        </div>
      )}
      {shot && (
        <div className="computer-teach">
          {t ? <>
            {/* The bot you're teaching watches you do it, then writes it up. */}
            <Avatar id={forBot} name={bots.find(b => b.id === forBot)?.name ?? ''} size={24} alive mood={teach.busy ? 'writing' : 'listening'} beat={t.steps} />
            <span className="dot live rec" /><b>Recording</b><span className="hint">{t.seconds >= t.limit ? 'Time’s up' : mmss(t.seconds)} · {t.steps} {t.steps === 1 ? 'step' : 'steps'}</span>
            <span className="grow" />
            <label className="hint">For <select className="select" value={forBot} onChange={e => setForBot(Number(e.target.value))}>{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select></label>
            <label className="hint">in <select className="select" value={forChat} onChange={e => setForChat(Number(e.target.value))}>{chats.map(c => <option key={c.chat_id} value={c.chat_id}>{chatName(c)}</option>)}</select></label>
            <button className="btn btn-sm" disabled={teach.busy} onClick={() => teachDo('stop')}>{teach.busy ? 'Drafting…' : 'Done, draft a skill'}</button>
            <button className="btn btn-sm btn-ghost" disabled={teach.busy} onClick={() => teachDo('discard')}>Discard</button>
          </> : <>
            {teach.done ? <Avatar id={forBot} name={bots.find(b => b.id === forBot)?.name ?? ''} size={24} mood="happy" /> : <Icon name="bolt" size={14} />}<span className="grow"><b>Teach by showing.</b> <span className="hint">Do a task once (up to 10 minutes) and a bot writes it up as a skill for you to check. Passwords you type aren’t kept.</span></span>
            <button className="btn btn-sm" onClick={() => teachDo('start')}>Start showing</button>
          </>}
          {(teach.done || teach.err) && <span className={teach.err ? 'error' : 'hint'} style={{ flexBasis: '100%' }}>{teach.err ?? teach.done}</span>}
        </div>
      )}
      {shot ? (
        <div className={'computer-screen' + (focus ? ' focused' : '')} tabIndex={0} onFocus={() => setFocus(true)} onBlur={() => setFocus(false)} onKeyDown={onKey}
          onWheel={e => input({ kind: 'scroll', ...at(e), direction: e.deltaY > 0 ? 'down' : 'up', amount: Math.min(10, Math.max(1, Math.round(Math.abs(e.deltaY) / 60))) })}>
          <img ref={img} src={`data:image/jpeg;base64,${shot.image}`} alt="The bots' computer screen" draggable={false}
            onClick={e => input({ kind: 'click', ...at(e), button: 'left' })}
            onDoubleClick={e => input({ kind: 'click', ...at(e), button: 'left', double: true })}
            onContextMenu={e => { e.preventDefault(); input({ kind: 'click', ...at(e), button: 'right' }) }} />
          <div className="computer-hint">{focus ? 'Taking over: typing goes to the computer. Click outside to stop.' : 'Click the screen to take over.'}</div>
        </div>
      ) : !err && <Empty icon="laptop" title="Starting the computer…">The screen shows up here in a moment.</Empty>}
    </>
  )
}

/** Desktop only: the app is the bots' home, so it should be running. */
/** When routines in the digest lane deliver their one message of the day. */
function DigestSettings() {
  const [at, setAt] = useState<string | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [weekly, setWeekly] = useState<boolean | null>(null)
  const [quiet, setQuiet] = useState<string | null>(null)
  useEffect(() => {
    invoke<string>('digest_time').then(setAt, () => {})
    invoke<boolean>('weekly_review').then(setWeekly, () => {})
    invoke<string>('quiet_hours').then(setQuiet, () => {})
  }, [])
  const [qFrom, qTo] = quiet ? quiet.split('-') : ['22:00', '07:00']
  // On a rejected range (e.g. from = to), go back to what's saved instead of showing what was refused.
  const saveQuiet = (hours: string) => {
    const was = quiet
    setQuiet(hours)
    invoke<string>('quiet_hours', { hours }).then(setQuiet, e => { setQuiet(was); setMsg(String(e)) })
  }
  const save = (time: string) => { setAt(time); invoke<string>('digest_time', { time }).then(t => { setAt(t); setMsg('Saved') }, e => setMsg(String(e))) }
  return (
    <Group title="Daily digest" note="Routines set to “In my daily digest” are collected into one message a day, with a line for quiet checks that found nothing. Routines set to “Right away” still message you as soon as they run, outside quiet hours.">
      <Row icon={<Icon name="clock" size={15} />} title="Send my digest at" desc={msg ?? 'Once a day, to each chat that has digest routines.'}>
        {at !== null && <input className="input" type="time" aria-label="Digest time" style={{ width: 'auto' }} value={at} onChange={e => e.target.value && save(e.target.value)} />}
      </Row>
      <Row icon={<Icon name="moon" size={15} />} title="Quiet hours"
        desc={quiet ? 'Routines that would message you wait for your digest, and asks from routines wait until morning. Replies to your own messages still come.' : 'Off: routines message you whenever they run.'}>
        {quiet !== null && <>
          {quiet && <>
            <input className="input" type="time" aria-label="Quiet from" style={{ width: 'auto' }} value={qFrom} onChange={e => e.target.value && saveQuiet(`${e.target.value}-${qTo}`)} />
            <span className="hint">to</span>
            <input className="input" type="time" aria-label="Quiet until" style={{ width: 'auto' }} value={qTo} onChange={e => e.target.value && saveQuiet(`${qFrom}-${e.target.value}`)} />
          </>}
          <Switch label="Quiet hours" checked={!!quiet} onChange={v => saveQuiet(v ? '22:00-07:00' : '')} />
        </>}
      </Row>
      <Row icon={<Icon name="memory" size={15} />} title="Weekly “what I learned”" desc="Once a week at that time, each Telegram, Slack or Discord chat gets a list of what your bots learned there on their own. Reply /undo with a number to take one back.">
        {weekly !== null && <Switch label="Weekly what I learned" checked={weekly} onChange={v => { setWeekly(v); invoke<boolean>('weekly_review', { on: v }).then(setWeekly, () => setWeekly(!v)) }} />}
      </Row>
    </Group>
  )
}

type Caps = { routine: number | null; bot: number | null; day: number | null }
type Windows = { at: number; status: string; windows: { name: string; used: number; resets_at: number }[] }
const WINDOW_NAME: Record<string, string> = { five_hour: '5-hour window', seven_day: 'Weekly', seven_day_opus: 'Weekly (Opus)', seven_day_sonnet: 'Weekly (Sonnet)' }

/** Daily caps on what routines may use, and the AI plan's own windows where the AI reports them. */
const KIND_NAME: Record<string, string> = { claude: 'Claude', codex: 'ChatGPT / Codex', grok: 'Grok', api: 'API key' }

function LimitsSettings() {
  const [caps, setCaps] = useState<Caps | null>(null)
  const [win, setWin] = useState<Windows | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [models, setModels] = useState<{ kind: string; model: string }[]>([])
  useEffect(() => {
    invoke<{ kind: string; model: string }[]>('routine_model').then(setModels, () => {})
    invoke<Caps>('caps').then(setCaps, () => {})
    invoke<Windows | null>('ai_windows').then(setWin, () => {})
  }, [])
  const save = (k: keyof Caps, v: string) => invoke<Caps>('caps', { [k]: v.trim() ? Math.max(0, Math.round(Number(v) * 1000)) : 0 }).then(c => { setCaps(c); setMsg(null) }, e => setMsg(String(e)))
  const field = (k: keyof Caps, title: string, desc: string) => (
    <Row icon={<Icon name="activity" size={15} />} title={title} desc={desc}>
      {caps && <><input className="input mono" type="number" min={0} step={10} aria-label={title} placeholder="No cap" style={{ width: 96 }}
        defaultValue={caps[k] ? caps[k]! / 1000 : ''} onBlur={e => save(k, e.target.value)} /><span className="hint">k tokens a day</span></>}
    </Row>
  )
  return (
    <Group title="Limits" note={msg ?? 'Caps on what routines use in a day. One that would go over pauses until midnight, and Needs you asks whether to let it go on today. Replies to your own messages are never stopped. Tokens read back from the AI’s cache count a tenth, as they’re billed; each routine shows what it uses on average.'}>
      {field('routine', 'Each routine', 'What one routine may use in a day.')}
      {field('bot', 'Each bot', 'What one bot may use in a day, replies included; only its routines pause.')}
      {field('day', 'All bots together', 'Everything, every bot, in a day.')}
      {models.map(m => (
        <Row key={m.kind} icon={<Icon name="wand" size={15} />} title={models.length > 1 ? `Model for routines (${KIND_NAME[m.kind] ?? m.kind})` : 'Model for routines'}
          desc="A lighter model of the same account (e.g. haiku, gpt-5-mini) makes every routine cheaper, and your plan's windows last longer. Use a routine's Test run to see if it's good enough. Blank: the bot's own model.">
          <input className="input" style={{ width: 170 }} placeholder="Same as the bot" aria-label={`Model for routines (${m.kind})`} defaultValue={m.model}
            onBlur={e => e.target.value.trim() !== m.model && invoke<{ kind: string; model: string }[]>('routine_model', { kind: m.kind, model: e.target.value }).then(v => { setModels(v); setMsg(null) }, er => setMsg(String(er)))} />
        </Row>
      ))}
      {win && win.windows.length > 0 && (
        <Row icon={<Icon name="clock" size={15} />} title="Your Claude plan"
          desc={`${win.windows.map(w => `${WINDOW_NAME[w.name] ?? w.name.replace(/_/g, ' ')}: ${Math.round(w.used * 100)}% used, resets ${fullTime(w.resets_at)}`).join(' · ')}. As Claude Code reported it ${ago(win.at)}; when a window is used up, routines wait until it resets.`} />
      )}
    </Group>
  )
}

function AppSettings() {
  const [on, setOn] = useState<boolean | null>(null)
  const [awake, setAwake] = useState<boolean | null>(null)
  const [tidyModel, setTidyModel] = useState<string | null>(null)
  useEffect(() => { invoke<{ model: string }>('tidy_model').then(r => setTidyModel(r.model), () => setTidyModel('')) }, [])
  const [keys, setKeys] = useState<string | null | undefined>(undefined)
  const [err, setErr] = useState<string | null>(null)
  useEffect(() => {
    invoke<boolean>('open_at_login').then(setOn, e => setErr(String(e)))
    invoke<boolean>('keep_awake').then(setAwake, () => {})
    invoke<string | null>('quick_shortcut').then(setKeys, () => setKeys(null))
  }, [])
  return (
    <Group title="General" note={err ?? `Your bots run while Mimir is open. Closing the window keeps it in the ${trayName}; quit from its ${trayName} icon.`}>
      <Row icon={<Icon name="laptop" size={15} />} title="Open Mimir when you log in">
        <Switch label="Open Mimir when you log in" checked={!!on} disabled={on === null} onChange={v => invoke<boolean>('open_at_login', { enabled: v }).then(setOn, er => setErr(String(er)))} />
      </Row>
      <Row icon={<Icon name="moon" size={15} />} title="Keep this computer awake"
        desc="So routines run on time while you're away. The screen still turns off, and closing a laptop still puts it to sleep.">
        <Switch label="Keep this computer awake" checked={!!awake} disabled={awake === null} onChange={v => invoke<boolean>('keep_awake', { enabled: v }).then(setAwake, er => setErr(String(er)))} />
      </Row>
      <Row icon={<Icon name="wand" size={15} />} title="Model for the memory tidy-up"
        desc="The daily review of your memory runs on the first bot's account. A cheaper model (e.g. haiku, gpt-5-nano) saves money on API keys. Blank: the bot's own model.">
        <input className="input" style={{ width: 170 }} placeholder="Same as the bot" aria-label="Model for the memory tidy-up" value={tidyModel ?? ''} disabled={tidyModel === null}
          onChange={e => setTidyModel(e.target.value)} onBlur={e => invoke<{ model: string }>('tidy_model', { model: e.target.value }).then(r => setTidyModel(r.model), er => setErr(String(er)))} />
      </Row>
      <Row icon={<Icon name="bolt" size={15} />} title="Quick ask"
        desc={keys ? 'Ask a bot from any app without opening this window.' : keys === null ? 'Its shortcuts are taken by other apps.' : 'Checking…'}>
        {keys && <kbd className="keys">{keys}</kbd>}
      </Row>
    </Group>
  )
}

type UpdateStatus = { current: string; checking: boolean; ready: { version: string; notes?: string | null } | null; error: string | null; checked_at: number; auto: boolean }

/** The app's own updates (always this computer's app, even when it shows a server): a newer version downloads in the
 *  background and waits here for a restart, which you choose, since it stops your bots for a moment. */
function UpdateSettings() {
  const st = useLoad(() => localInvoke<UpdateStatus>('update_status'), [], 5000)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const s = st.data
  const check = () => { setBusy(true); setErr(null); localInvoke<UpdateStatus>('update_check').then(() => st.reload(), e => setErr(String(e))).finally(() => setBusy(false)) }
  const restart = () => { setBusy(true); setErr(null); localInvoke('update_install').catch(e => { setErr(String(e)); setBusy(false) }) }
  const state = !s ? '' : s.ready ? `Version ${s.ready.version} is ready. Restarting takes a few seconds; your bots pick up where they left off.`
    : s.checking || busy ? 'Checking…'
    : s.error ? "Couldn't check for updates just now."
    : s.checked_at ? `Up to date (checked ${ago(s.checked_at)}).` : 'Mimir checks a minute after it starts, then every 6 hours.'
  return (
    <Group title="Updates" note={err ?? (s?.error && !s.ready ? s.error : undefined)}>
      <Row icon={<Icon name="retry" size={15} />} title={s ? `Mimir ${s.current}` : 'Mimir'} desc={state}>
        {s?.ready
          ? <button className="btn btn-primary btn-sm" disabled={busy} onClick={restart}>Restart to update</button>
          : <button className="btn btn-sm" disabled={!s || s.checking || busy} onClick={check}>Check now</button>}
      </Row>
      <Row icon={<Icon name="clock" size={15} />} title="Check for updates automatically" desc="Updates download in the background; nothing restarts until you say so.">
        <Switch label="Check for updates automatically" checked={!!s?.auto} disabled={!s} onChange={v => localInvoke<UpdateStatus>('update_auto', { on: v }).then(() => st.reload(), e => setErr(String(e)))} />
      </Row>
    </Group>
  )
}

const KEY_ROWS = [
  { kind: 'openrouter', title: 'OpenRouter', placeholder: 'sk-or-…', hint: <>Create one at <a href="https://openrouter.ai/keys" onClick={openLink}>openrouter.ai/keys</a>.</> },
  { kind: 'xai', title: 'xAI (Grok)', placeholder: 'xai-…', hint: <>Create one at <a href="https://console.x.ai" onClick={openLink}>console.x.ai</a>.</> },
  { kind: 'openai', title: 'OpenAI-compatible API key', placeholder: 'sk-…', hint: 'For bots on the “API key” account: OpenAI, or the service at the bot’s endpoint.' },
]

function ApiKeySettings() {
  const [has, setHas] = useState<Record<string, boolean>>({})
  const [grok, setGrok] = useState<Account | null>(null)
  const [chatgpt, setChatgpt] = useState<Account | null>(null)
  const loadGrok = () => invoke<Record<string, Account>>('accounts').then(a => { setGrok(a?.grok ?? null); setChatgpt(a?.chatgpt ?? null) }, () => {})
  useEffect(() => { loadGrok() }, [])
  const [open, setOpen] = useState<string | null>(null)
  const [key, setKey] = useState('')
  const [msg, setMsg] = useState<Record<string, string>>({})
  const [err, setErr] = useState<string | null>(null)
  const load = () => invoke<Record<string, boolean>>('api_keys').then(h => setHas(h ?? {}), e => setErr(String(e)))
  useEffect(() => { load() }, [])
  const save = (kind: string, k: string) => invoke('set_api_key', { kind, key: k }).then(load)
    .then(() => { setKey(''); setOpen(null); setErr(null); setMsg({ ...msg, [kind]: k ? 'Key saved' : 'Key removed' }) }, e => setErr(String(e)))
  return (
    <Group title="AI keys" note={err ? <span className="error">{err}</span> : `Shared by all your bots on that account. Stored in ${SECRETS_AT}.`}>
      {(() => {
        const row = (control: React.ReactNode) => <Row icon={<Icon name="user" size={15} />} title="ChatGPT plan"
          desc={!chatgpt ? 'Checking…' : chatgpt.signed_in ? `You’re using your ChatGPT plan (${chatgpt.account}). Set how much Mimir may use each week in ChatGPT’s settings.` : 'Sign in to run bots on your ChatGPT Plus or Pro plan, with nothing to install'}>{control}</Row>
        if (!chatgpt) return row(null)
        if (!chatgpt.signed_in) return <SignInChatGPT onDone={loadGrok} place={(control, below) => <>{row(control)}{below}</>} />
        return row(<>
          <a className="btn btn-sm btn-ghost" href="https://chatgpt.com/#settings" onClick={openLink}>Manage usage</a>
          <button className="btn btn-sm btn-ghost btn-danger" onClick={() => invoke('chatgpt_sign_out').then(loadGrok, e => setErr(String(e)))}>Sign out</button>
        </>)
      })()}
      <Row icon={<Icon name="user" size={15} />} title="Grok subscription"
        desc={!grok ? 'Checking…' : !grok.installed ? 'Install Grok Build to use your SuperGrok or X Premium+ plan' : grok.signed_in ? 'Signed in for Mimir' : 'Not signed in'}>
        {grok?.installed && !grok.signed_in && <SignInGrok onDone={loadGrok} />}
      </Row>
      {KEY_ROWS.map(r => (
        <Fragment key={r.kind}>
          <Row icon={<Icon name="key" size={15} />} title={r.title} desc={msg[r.kind] ?? (has[r.kind] ? 'A key is saved' : 'No key saved')}>
            {has[r.kind] && <button className="btn btn-sm btn-ghost btn-danger" onClick={() => save(r.kind, '')}>Remove</button>}
            {r.kind === 'openrouter' && !has[r.kind] && <ConnectOpenRouter onDone={() => { load(); setMsg({ ...msg, openrouter: 'Connected' }) }}>Connect</ConnectOpenRouter>}
            <button className="btn btn-sm" onClick={() => { setOpen(open === r.kind ? null : r.kind); setKey('') }}>{open === r.kind ? 'Cancel' : has[r.kind] ? 'Change' : 'Add key'}</button>
          </Row>
          {open === r.kind && (
            <form className="row-block" onSubmit={e => { e.preventDefault(); if (key.trim()) save(r.kind, key) }}>
              <div className="editor" style={{ margin: 0 }}><div className="er">
                <input className="input" type="password" placeholder={r.placeholder} value={key} onChange={e => setKey(e.target.value)} autoComplete="off" aria-label={r.title} autoFocus />
                <button className="btn" disabled={!key.trim()}>Save key</button>
              </div></div>
              <span className="hint">{r.hint}</span>
            </form>
          )}
        </Fragment>
      ))}
    </Group>
  )
}

/** Your email for the bots, read-only (email_search, email_read): IMAP with an app password. Saving signs in once. */
function EmailSettings() {
  const st = useLoad(() => invoke<{ host: string; user: string; smtp?: string; smtp_port?: number } | null>('email_settings', {}), [])
  const [d, setD] = useState({ host: '', port: '993', user: '', password: '', smtp: '', smtpPort: '' })
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const save = () => {
    setBusy(true); setErr(null)
    invoke<{ messages: number }>('email_settings', { host: d.host.trim(), port: Number(d.port) || 993, user: d.user.trim(), password: d.password, smtpHost: d.smtp.trim(), smtpPort: Number(d.smtpPort) || undefined })
      .then(r => { setMsg(`Signed in · ${r.messages} messages in your inbox`); setD({ host: '', port: '993', user: '', password: '', smtp: '', smtpPort: '' }); return st.reload() }, e => setErr(String(e)))
      .finally(() => setBusy(false))
  }
  const remove = () => invoke('email_settings', { host: '' }).then(() => { setMsg(null); return st.reload() }, e => setErr(String(e)))
  return (
    <Group title="Email" note={err ? <span className="error">{err}</span> : `Lets your bots search and read your email in a chat with you, never in a group, write drafts, and send an email when you approve it (every one asks you, at most 20 a day). Nothing is marked read, moved or deleted. Emails count as outside content. The password is stored in ${SECRETS_AT}.`}>
      {st.data ? (
        <Row icon={<Icon name="inbox" size={15} />} title={st.data.user} desc={msg ?? `Read through ${st.data.host}${st.data.smtp ? ` · sent through ${st.data.smtp}${st.data.smtp_port && st.data.smtp_port !== 465 ? `:${st.data.smtp_port}` : ''}` : ''}`}>
          <button className="btn btn-sm btn-ghost btn-danger" onClick={remove}>Remove</button>
        </Row>
      ) : (
        <form className="row-block" onSubmit={e => { e.preventDefault(); save() }}>
          <div className="editor" style={{ margin: 0 }}>
            <div className="er">
              <input className="input" placeholder="IMAP server, e.g. imap.gmail.com" aria-label="IMAP server" value={d.host} onChange={e => setD({ ...d, host: e.target.value })} />
              <input className="input" style={{ width: 80 }} aria-label="Port" value={d.port} onChange={e => setD({ ...d, port: e.target.value })} />
              <input className="input" placeholder="Outgoing server (optional)" aria-label="Outgoing server" value={d.smtp} onChange={e => setD({ ...d, smtp: e.target.value })} />
              <input className="input" style={{ width: 80 }} placeholder="465" inputMode="numeric" aria-label="Outgoing port" title="Outgoing port: 465, or 587 if your provider says STARTTLS" value={d.smtpPort} onChange={e => setD({ ...d, smtpPort: e.target.value })} />
            </div>
            <div className="er">
              <input className="input" placeholder="Your address" aria-label="Email address" autoComplete="off" value={d.user} onChange={e => setD({ ...d, user: e.target.value })} />
              <input className="input" type="password" placeholder="App password" aria-label="App password" autoComplete="off" value={d.password} onChange={e => setD({ ...d, password: e.target.value })} />
              <button className="btn" disabled={busy || !d.host.trim() || !d.user.trim() || !d.password}>{busy ? 'Signing in…' : 'Save'}</button>
            </div>
          </div>
          <span className="hint">Use an app password, not your normal one: Gmail and iCloud make one under your account's security settings (two-step sign-in has to be on). Gmail is imap.gmail.com, iCloud imap.mail.me.com, Fastmail imap.fastmail.com. (Outlook and Microsoft 365 mail need Microsoft's own sign-in, which Mimir doesn't do yet.)</span>
        </form>
      )}
    </Group>
  )
}

type CalDav = { name?: string; user?: string; url?: string; server?: string; guess?: string; calendars?: { url: string; name: string }[] } | null

/** Adding events to your calendar (calendar_create) over CalDAV: iCloud, Fastmail, Nextcloud. Reading is the link in Meetings. */
function CalendarSettings() {
  const st = useLoad(() => invoke<CalDav>('caldav_settings', {}), [])
  const [d, setD] = useState<{ server: string; user: string; password: string } | null>(null)
  const [cals, setCals] = useState<{ url: string; name: string }[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const s = st.data
  const form = d ?? { server: s?.guess ?? '', user: s?.user ?? '', password: '' }
  const save = (args: { server: string; user: string; password?: string; calendar?: string }, list = false) => {
    setBusy(true); setErr(null)
    return invoke<CalDav>('caldav_settings', args).then(r => { setD(null); setCals(r?.calendars && (list || r.calendars.length > 1) ? r.calendars : null); return st.reload() }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const connected = !!s?.name
  return (
    <Group title="Calendar" note={err ? <span className="error">{err}</span> : 'Lets your bots add events to your calendar in a chat with you: each one asks first, showing when, what and where (“Always” works here). Bots read your calendar from its link in Meetings. Google Calendar needs Google’s own sign-in for this, so bots give you add-to-calendar links there instead.'}>
      {connected ? (
        <Row icon={<Icon name="clock" size={15} />} title={`Adds events to ${s!.name}`} desc={`${s!.user}${s!.server ? ` · ${s!.server}` : ''}`}>
          {cals
            ? <select className="input" style={{ width: 180 }} aria-label="Calendar" value={s!.url} disabled={busy} onChange={e => save({ server: s!.server ?? '', user: s!.user ?? '', calendar: e.target.value }, true)}>{cals.map(c => <option key={c.url} value={c.url}>{c.name}</option>)}</select>
            : <button className="btn btn-sm btn-ghost" disabled={busy} onClick={() => save({ server: s!.server ?? '', user: s!.user ?? '', calendar: s!.url }, true)}>{busy ? 'Looking…' : 'Other calendar'}</button>}
          <button className="btn btn-sm btn-ghost btn-danger" onClick={() => invoke('caldav_settings', { server: '' }).then(() => { setCals(null); return st.reload() }, e => setErr(String(e)))}>Remove</button>
        </Row>
      ) : (
        <form className="row-block" onSubmit={e => { e.preventDefault(); save({ server: form.server.trim(), user: form.user.trim(), password: form.password }) }}>
          <div className="editor" style={{ margin: 0 }}>
            <div className="er">
              <input className="input" placeholder="CalDAV server, e.g. https://caldav.icloud.com" aria-label="CalDAV server" value={form.server} onChange={e => setD({ ...form, server: e.target.value })} />
              <input className="input" placeholder="User name (your address)" aria-label="Calendar user name" autoComplete="off" value={form.user} onChange={e => setD({ ...form, user: e.target.value })} />
            </div>
            <div className="er">
              <input className="input" type="password" placeholder={s?.user && form.user.trim().toLowerCase() === s.user.toLowerCase() ? 'App password (empty: the email one)' : 'App password'} aria-label="Calendar app password" autoComplete="off" value={form.password} onChange={e => setD({ ...form, password: e.target.value })} />
              <button className="btn" disabled={busy || !form.server.trim() || !form.user.trim()}>{busy ? 'Signing in…' : 'Connect'}</button>
            </div>
          </div>
          <span className="hint">iCloud is https://caldav.icloud.com and Fastmail https://caldav.fastmail.com, with the same app password as their email. Nextcloud is https://your-server/remote.php/dav.</span>
        </form>
      )}
    </Group>
  )
}

/** Once email or a calendar is connected: one read-only look at both, whose findings wait in Memory as suggestions. */
function KnowMeSettings() {
  const st = useLoad(() => invoke<{ ready: boolean; done: boolean }>('know_me', { chatId: LOCAL }), [], 4000)
  const [state, setState] = useState<{ busy?: boolean; found?: number; err?: string }>({})
  if (!st.data?.ready || (st.data.done && state.found === undefined)) return null
  const go = () => { setState({ busy: true }); invoke<number>('know_me', { chatId: LOCAL, run: true }).then(found => { setState({ found }); st.reload() }, e => setState({ err: String(e) })) }
  return (
    <Group title="Get to know me" note={state.err ? <span className="error">{state.err}</span> : 'Once, and read-only: a bot looks at who wrote to you this month and what’s on your calendar (senders, subjects and events, never what an email says) and suggests what to remember. Nothing is kept until you accept it in Memory, and what you accept stays marked as found, not told.'}>
      <Row icon={<Icon name="memory" size={15} />} title={state.found !== undefined ? (state.found ? `Found ${state.found} thing${state.found === 1 ? '' : 's'} to suggest` : 'Nothing worth suggesting') : 'Let your bots get to know you'}
        desc={state.found !== undefined ? (state.found ? 'Accept or dismiss each in Memory (built-in chat).' : 'Tell a bot about yourself instead.') : 'Uses your connected email and calendar link'}>
        {state.found === undefined && <button className="btn btn-sm" disabled={state.busy} onClick={go}>{state.busy ? 'Looking…' : 'Get to know me'}</button>}
      </Row>
    </Group>
  )
}

/** Web search for every bot (web_search): Brave Search's key, or your own SearXNG. Claude's own search works without. */
function WebSearchSettings() {
  const st = useLoad(() => invoke<{ brave: boolean; searxng_url: string }>('search_settings', {}), [])
  const [key, setKey] = useState('')
  const [url, setUrl] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const done = () => { setErr(null); return st.reload() }
  const s = st.data
  return (
    <Group title="Web search" note={err ? <span className="error">{err}</span> : 'Lets every bot search the web, whatever its AI account. Results count as outside content, and a search that would carry one of your memories asks first.'}>
      <Row icon={<Icon name="search" size={15} />} title="Brave Search"
        desc={s?.brave ? 'A key is saved; bots search with Brave' : <>A free key from <a href="https://brave.com/search/api/" onClick={openLink}>brave.com/search/api</a>.</>}>
        {s?.brave ? <button className="btn btn-sm btn-ghost btn-danger" onClick={() => invoke('set_api_key', { kind: 'brave', key: '' }).then(done, e => setErr(String(e)))}>Remove</button>
          : <form className="er" style={{ gap: 8 }} onSubmit={e => { e.preventDefault(); if (key.trim()) invoke('set_api_key', { kind: 'brave', key: key.trim() }).then(() => { setKey(''); return done() }, er => setErr(String(er))) }}>
              <input className="input" type="password" style={{ width: 200 }} placeholder="Brave Search API key" aria-label="Brave Search API key" value={key} onChange={e => setKey(e.target.value)} autoComplete="off" />
              <button className="btn btn-sm" disabled={!key.trim()}>Save</button>
            </form>}
      </Row>
      <Row icon={<Icon name="server" size={15} />} title="Or your own SearXNG" desc={s?.brave ? 'Used when no Brave key is saved.' : 'Its address, with JSON results switched on (search.formats: json).'}>
        <input className="input" style={{ width: 230 }} placeholder="https://search.example.com" aria-label="SearXNG address" disabled={!s}
          value={url ?? s?.searxng_url ?? ''} onChange={e => setUrl(e.target.value)}
          onBlur={() => url !== null && invoke('search_settings', { searxngUrl: url }).then(() => { setUrl(null); return done() }, e => setErr(String(e)))} />
      </Row>
    </Group>
  )
}

function BrowserSettings({ onChange }: { onChange: () => void }) {
  const servers = useLoad(() => invoke<{ servers: { name: string }[] }>('tool_servers'), [], 60000)
  const vault = useLoad(() => invoke<VaultEntry[]>('vault'), [], 60000)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [draft, setDraft] = useState({ name: '', value: '', domains: '' })
  const on = !!servers.data?.servers.some(s => s.name === 'browser')
  const toggle = (v: boolean) => {
    setBusy(true); setErr(null)
    invoke('browser_enable', { enabled: v }).then(() => { servers.reload(); onChange() }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const save = () => invoke('vault_save', {
    entry: { name: draft.name.trim().toUpperCase(), domains: draft.domains.split(/[,\s]+/).filter(Boolean) }, value: draft.value,
  }).then(() => { setDraft({ name: '', value: '', domains: '' }); setErr(null); vault.reload() }, e => setErr(String(e)))
  return (
    <Group title="Browser" note="Opening a new page asks you first. Running page scripts is off. Needs Node.js; the first time downloads Chromium (~100 MB).">
      <Row icon={<Icon name="browser" size={15} />} title="Web browser" desc={busy ? (on ? 'Turning off…' : 'Setting up — downloading Chromium the first time…') : 'A real headless browser bots can use: open pages, click, fill in forms.'}>
        <Switch label="Web browser" checked={on} disabled={busy} onChange={toggle} />
      </Row>
      {on && (
        <>
          <Row icon={<Icon name="shield" size={15} />} title="Saved logins" desc="Bots type these by name; the browser fills in the real value, which the AI never sees. Each works only on its sites, and you approve every use." />
          {(vault.data ?? []).map(v => (
            <Row key={v.name} title={<code>{v.name}</code>} desc={v.domains.join(', ')}>
              <button className="btn btn-sm btn-ghost btn-danger icon-btn" aria-label={`Remove ${v.name}`} title="Remove" onClick={() => invoke('vault_delete', { name: v.name }).then(vault.reload, e => setErr(String(e)))}><Icon name="trash" size={14} /></button>
            </Row>
          ))}
          <form className="row-block" onSubmit={e => { e.preventDefault(); save() }}>
            <div className="editor" style={{ margin: 0 }}><div className="er">
              <input className="input mono" aria-label="Login name" placeholder="GITHUB_PASSWORD" value={draft.name} onChange={e => setDraft({ ...draft, name: e.target.value })} />
              <input className="input" aria-label="Secret value" type="password" placeholder="Password or token" value={draft.value} onChange={e => setDraft({ ...draft, value: e.target.value })} autoComplete="new-password" />
              <input className="input" aria-label="Allowed sites" placeholder="github.com" value={draft.domains} onChange={e => setDraft({ ...draft, domains: e.target.value })} />
              <button className="btn" disabled={!draft.name.trim() || !draft.value || !draft.domains.trim()}>Save</button>
            </div></div>
          </form>
        </>
      )}
      {err && <div className="row-block"><p className="error" style={{ margin: 0 }}>{err}</p></div>}
    </Group>
  )
}

type ToolServer = { name: string; preset: string | null; command: string; args: string[]; url: string; env: string[]; headers: string[]; tools: number }
type EnvRow = { key: string; value: string }
/** `url` set = a remote server, and `env` holds its headers instead. */
type ParsedServer = { command: string; args: string[]; url: string; name: string; env: EnvRow[] }

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '')
/** A name from the package or image: "@modelcontextprotocol/server-filesystem" → "filesystem". */
function guessName(command: string, args: string[]) {
  const pkg = args.find(a => !a.startsWith('-') && /[/@]|mcp|server/i.test(a)) ?? args.find(a => !a.startsWith('-')) ?? command
  return slug(pkg.split('/').pop()!.replace(/@[^@]*$/, '').replace(/^mcp-server-|^server-|^mcp-|-mcp-server$|-mcp$|-server$/g, '')) || 'tools'
}
/** "https://mcp.deepwiki.com/mcp" → "deepwiki". */
const urlName = (url: string) => { try { return slug(new URL(url).hostname.replace(/^(mcp|api|www)\./, '').split('.')[0]) || 'tools' } catch { return 'tools' } }

/** What people paste from an MCP server's instructions: a command line ("KEY=v npx -y pkg ~/dir"), a URL,
 *  or a JSON config ({"mcpServers": {"name": {command, args, env} or {url, headers}}}, the same without the wrapper, or one entry). */
function parseServer(text: string): ParsedServer | string | null {
  const t = text.trim()
  if (!t) return null
  if (t.startsWith('{') || t.startsWith('"')) {
    let v: any
    try { v = JSON.parse(t.startsWith('"') ? `{${t.replace(/,\s*$/, '')}}` : t) } catch { return "That looks like a JSON config, but it doesn't parse." }
    v = v.mcpServers ?? v.servers ?? v
    const [name, c]: [string, any] = v.command || v.url ? ['', v] : (Object.entries(v)[0] as [string, any] | undefined) ?? ['', null]
    if (!c || typeof c !== 'object') return 'No server found in that config.'
    const rows = (o: object | undefined) => Object.entries(o ?? {}).map(([key, value]) => ({ key, value: String(value) }))
    if (c.type === 'sse') return 'That server uses the older SSE transport, which isn’t supported. Look for its /mcp (Streamable HTTP) address instead.'
    if (c.url) return /^https?:\/\//.test(c.url) ? { command: '', args: [], url: String(c.url), name: slug(name) || urlName(c.url), env: rows(c.headers) } : 'The "url" needs to start with https://.'
    if (!c.command) return 'The config needs a "command" or a "url".'
    const args = (c.args ?? []).map(String)
    return { command: String(c.command), args, url: '', name: slug(name) || guessName(c.command, args), env: rows(c.env) }
  }
  if (/^https?:\/\/\S+$/.test(t)) return { command: '', args: [], url: t, name: urlName(t), env: [] }
  const words = [...t.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map(m => m[1] ?? m[2] ?? m[3])
  const env: EnvRow[] = []
  while (words.length > 1 && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0])) {
    const [key, ...rest] = words.shift()!.split('=')
    env.push({ key, value: rest.join('=') })
  }
  const [command, ...args] = words
  return { command, args, url: '', name: guessName(command, args), env }
}

function ToolServerSettings({ onChange }: { onChange: () => void }) {
  const list = useLoad(() => invoke<{ servers: ToolServer[] }>('tool_servers'), [], 60000)
  const [form, setForm] = useState<{ text: string; name: string; guess: string; env: EnvRow[]; editing: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const servers = (list.data?.servers ?? []).filter(s => !s.preset)
  const here = inDesktop && !remoteServer() ? 'this computer' : 'your server'
  const parsed = form ? parseServer(form.text) : null
  const remote = !!parsed && typeof parsed !== 'string' && !!parsed.url
  const quote = (a: string) => /[\s"]/.test(a) ? `'${a}'` : a
  const open = (s?: ToolServer) => {
    setErr(null); setMsg(null)
    setForm(s ? { text: s.url || [s.command, ...s.args].map(quote).join(' '), name: s.name, guess: '', env: (s.url ? s.headers : s.env).map(key => ({ key, value: '' })), editing: true }
      : { text: '', name: '', guess: '', env: [], editing: false })
  }
  // Pasting fills in the name (until you change it) and any env variables the config lists.
  const type = (text: string) => setForm(f => {
    if (!f) return f
    const p = parseServer(text)
    if (!p || typeof p === 'string') return { ...f, text }
    return { ...f, text, name: f.editing || f.name !== f.guess ? f.name : p.name, guess: p.name, env: p.env.length ? p.env : f.env }
  })
  const setEnv = (i: number, patch: Partial<EnvRow>) => setForm(f => f && { ...f, env: f.env.map((e, j) => j === i ? { ...e, ...patch } : e) })
  const save = async () => {
    if (!form || !parsed || typeof parsed === 'string') return
    setBusy(true); setErr(null)
    try {
      const rows = Object.fromEntries(form.env.filter(e => e.key.trim()).map(e => [e.key.trim(), e.value]))
      const n = await invoke<number>('tool_server_save', { server: { name: form.name.trim(), command: parsed.command, args: parsed.args, url: parsed.url, ...parsed.url ? { headers: rows } : { env: rows } } })
      setMsg(`${form.name.trim()} is ready with ${n} tool${n === 1 ? '' : 's'}.`); setForm(null); list.reload(); onChange()
    } catch (e) { setErr(String(e)) } finally { setBusy(false) }
  }
  const remove = (s: ToolServer) => ask(`Remove ${s.name}? Your bots lose its tools.`, 'Remove', true).then(ok => { if (ok) void invoke('tool_server_delete', { name: s.name }).then(() => { setMsg(null); list.reload(); onChange() }, e => setErr(String(e))) })

  return (
    <Group title="Tool servers" note={<>Give your bots more abilities with <a href="https://github.com/modelcontextprotocol/servers" onClick={openLink}>MCP servers</a> that run on {here} or online: your files, GitHub, a database. Reading is allowed; anything that changes things asks you first (see Permissions). Tokens are stored in {SECRETS_AT}.</>}>
      {servers.map(s => (
        <Row key={s.name} icon={<Icon name="plug" size={15} />} title={s.name} desc={<>{s.tools} tool{s.tools === 1 ? '' : 's'} · <code>{s.url || [s.command, ...s.args].join(' ')}</code></>}>
          <button className="btn btn-sm" onClick={() => open(s)}>Edit</button>
          <button className="btn btn-sm btn-ghost btn-danger icon-btn" aria-label={`Remove ${s.name}`} title="Remove" onClick={() => remove(s)}><Icon name="trash" size={14} /></button>
        </Row>
      ))}
      {!form && (
        <Row icon={<Icon name="plus" size={15} />} title={servers.length ? 'Add another tool server' : 'Add a tool server'} desc={msg ?? 'Paste the setup line or JSON config from the server’s instructions.'}>
          <button className="btn btn-sm" onClick={() => open()}>Add</button>
        </Row>
      )}
      {form && (
        <form className="row-block" onSubmit={e => { e.preventDefault(); save() }}>
          <label className="field-label"><span>Command, URL or JSON config</span>
            <textarea className="textarea mono" rows={3} autoFocus={!form.editing} value={form.text} onChange={e => type(e.target.value)}
              placeholder={'npx -y @modelcontextprotocol/server-filesystem ~/Documents/Notes\n\nor paste {"mcpServers": {…}} from the server’s instructions'} /></label>
          {typeof parsed === 'string' && <span className="hint warn"><Icon name="alert" size={13} />{parsed}</span>}
          <label className="field-label"><span>Name</span>
            <input className="input mono" value={form.name} readOnly={form.editing} placeholder="notes" onChange={e => setForm({ ...form, name: slug(e.target.value) })} />
            <span className="hint">Tools show up as <code>{form.name || 'name'}__…</code> in Permissions and the run log.</span></label>
          <div className="field-label"><span>{remote ? 'Headers' : 'Environment variables'}</span>
            <div className="editor" style={{ margin: 0, display: 'grid', gap: 6 }}>
              {form.env.map((e, i) => (
                <div className="er" key={i}>
                  <input className="input mono" style={{ flex: 1 }} aria-label={remote ? 'Header name' : 'Variable name'} placeholder={remote ? 'Authorization' : 'GITHUB_TOKEN'} value={e.key} onChange={ev => setEnv(i, { key: ev.target.value })} />
                  <input className="input" style={{ flex: 1.4 }} type="password" aria-label={`Value of ${e.key || 'variable'}`} autoComplete="new-password"
                    placeholder={form.editing && !e.value ? 'Saved — leave empty to keep' : remote ? 'Bearer …' : 'Value'} value={e.value} onChange={ev => setEnv(i, { value: ev.target.value })} />
                  <button type="button" className="btn btn-sm btn-ghost icon-btn" aria-label="Remove variable" onClick={() => setForm({ ...form, env: form.env.filter((_, j) => j !== i) })}><Icon name="x" size={13} /></button>
                </div>
              ))}
            </div>
            <span><button type="button" className="btn btn-sm btn-ghost" onClick={() => setForm({ ...form, env: [...form.env, { key: '', value: '' }] })}><Icon name="plus" size={13} />{remote ? 'Add header' : 'Add variable'}</button></span>
          </div>
          <div className="actions">
            <button type="button" className="btn btn-ghost" onClick={() => { setForm(null); setErr(null) }}>Cancel</button>
            <span className="grow">{busy && <span className="hint">{remote ? 'Connecting once to see what it can do.' : 'Starting it once to see what it can do. The first time can take a minute.'}</span>}</span>
            <button className="btn btn-primary" disabled={busy || !parsed || typeof parsed === 'string' || !form.name.trim()}>{busy ? 'Starting…' : form.editing ? 'Save' : 'Add server'}</button>
          </div>
          {err && <p className="error" style={{ margin: 0 }}>{err}</p>}
        </form>
      )}
      {err && !form && <div className="row-block"><p className="error" style={{ margin: 0 }}>{err}</p></div>}
    </Group>
  )
}

/** A server doesn't update itself: whether it tells you when there's a new version (your chat, and Needs you). */
function ServerUpdates() {
  const st = useLoad(() => invoke<{ server: boolean; on: boolean; current: string; latest: string | null }>('update_notice', {}), [], 60000)
  const s = st.data
  if (!s?.server) return null
  const behind = s.latest && s.latest !== s.current
  return (
    <Row icon={<Icon name="retry" size={15} />} title="Tell me when there’s a new version"
      desc={`Runs ${s.current}${behind ? ` · ${s.latest} is out (see Needs you)` : ''}. Once per version, in your chat and in Needs you, with the commands to update.`}>
      <Switch label="Tell me when there’s a new version" checked={s.on} onChange={on => invoke('update_notice', { on }).then(st.reload, e => tell(String(e)))} />
    </Row>
  )
}

function ServerSettings() {
  const current = remoteServer()
  const [url, setUrl] = useState(current?.url ?? '')
  const [token, setToken] = useState('')
  const [err, setErr] = useState<string | null>(null)
  if (!inDesktop) return (
    <Group title="This server">
      <Row icon={<Icon name="server" size={15} />} title="Signed in" desc="Your bots and memory live on this server.">
        <button type="button" className="btn btn-sm" onClick={() => { setWebToken(null); location.reload() }}><Icon name="logout" size={13} />Sign out</button>
      </Row>
      <ServerUpdates />
    </Group>
  )
  return (
    <Group title="Your server" note={current ? undefined : <>Bots and memory on this computer aren’t copied. To move them, copy <code>~/.mimir/mimir.db</code> to the server first.</>}>
      <Row icon={<Icon name="server" size={15} />} title={current ? 'Connected to your server' : 'Run your bots on your own server'}
        desc={current ? <>Bots run on <code>{current.url}</code>, so they keep working while this computer sleeps. Bots on this computer are paused.</> : <>Run <code>mimir server</code> on a machine you own to keep bots online 24/7, then connect this app.</>}>
        {current && <button type="button" className="btn btn-sm" onClick={() => connectServer(null)}>Disconnect</button>}
      </Row>
      {!current && (
        <form className="row-block" onSubmit={e => {
          e.preventDefault()
          const s = { url: url.trim().replace(/\/$/, ''), token: token.trim() }
          const insecure = insecureServer(s.url)
          if (insecure) { setErr(insecure); return }
          probe(s.url, s.token).then(info => info.mode === 'server' ? connectServer(s) : setErr('That address is not a mimir server.'), e => setErr(String(e)))
        }}>
          <div className="editor" style={{ margin: 0 }}><div className="er">
            <input className="input" placeholder="https://bots.example.com" aria-label="Server address" value={url} onChange={e => setUrl(e.target.value)} />
            <input className="input" type="password" placeholder="Admin token" aria-label="Admin token" value={token} onChange={e => setToken(e.target.value)} autoComplete="off" />
            <button className="btn" disabled={!url.trim() || !token.trim()}>Connect</button>
          </div></div>
          {err && <p className="error" style={{ margin: 0 }}>{err}</p>}
        </form>
      )}
    </Group>
  )
}

const TOOL_LABEL: Record<string, string> = {
  memory_store: 'Remember things', memory_search: 'Look up memories', memory_forget: 'Forget a memory',
  routine_create: 'Create routines', routine_list: 'List routines', routine_delete: 'Delete a routine', web_fetch: 'Read web pages',
  chat_search: 'Search earlier conversations', meeting_search: 'Search your meetings', meeting_read: 'Read a meeting transcript', ask_bot: 'Hand tasks to other bots', skill_view: 'Read its skills', plan_propose: 'Propose a plan to approve once',
}

/** "browser__browser_click" → "Click" (grouped under Browser) */
const toolLabel = (t: string) => TOOL_LABEL[t] ?? (t.includes('__') ? t.split('__')[1].replace(/^browser_/, '').replaceAll('_', ' ').replace(/^./, c => c.toUpperCase()) : t)

function Permissions() {
  const rules = useLoad(() => invoke<Rule[]>('rules'), [], 60000)
  const [err, setErr] = useState<string | null>(null)
  const [openServer, setOpenServer] = useState<string | null>(null)
  const set = (tool: string, action: string) => invoke('set_rule', { tool, action }).then(() => { setErr(null); rules.reload() }, e => setErr(String(e)))
  const all = rules.data ?? []
  const builtin = all.filter(r => !r.tool.includes('__'))
  const byServer = new Map<string, Rule[]>()
  all.filter(r => r.tool.includes('__')).forEach(r => { const s = r.tool.split('__')[0]; byServer.set(s, [...(byServer.get(s) ?? []), r]) })
  const row = (r: Rule) => (
    <Row key={r.tool} title={toolLabel(r.tool)} desc={r.action !== r.default ? `Changed from “${r.default}”` : undefined}>
      <Segmented size="sm" label={toolLabel(r.tool)} value={r.action} onChange={a => set(r.tool, a)}
        options={[{ id: 'allow', label: 'Allow' }, { id: 'ask', label: 'Ask' }, { id: 'deny', label: 'Deny' }]} />
    </Row>
  )
  const sites = useLoad(() => invoke<{ tool: string; host: string; action: string }[]>('site_rules'), [], 60000)
  const removeSite = (tool: string, host: string) => invoke('set_site_rule', { tool, host, action: 'default' }).then(sites.reload, e => setErr(String(e)))
  const setAll = async (action: 'allow' | 'default') => {
    const msg = action === 'allow'
      ? 'Allow every tool without asking? Sending an email, making a bot and using a saved login still ask every time, and after a bot reads outside content, anything that changes your data still asks.'
      : 'Put every tool back to how Mimir comes (deleting and opening sites ask, the rest is allowed)?'
    if (!await ask(msg, action === 'allow' ? 'Allow all' : 'Back to defaults', action === 'allow')) return
    for (const r of all) if (r.action !== (action === 'default' ? r.default : 'allow')) await invoke('set_rule', { tool: r.tool, action }).catch(e => setErr(String(e)))
    rules.reload()
  }
  return (
    <Group title="Permissions" action={<span className="perm-all">
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => setAll('default')} disabled={all.every(r => r.action === r.default)}>Back to defaults</button>
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => setAll('allow')} disabled={!all.length || all.every(r => r.action === 'allow')}>Allow all</button>
    </span>} note={err ?? '“Ask” sends you Approve / Deny first. After a bot reads a web page, anything that changes your data asks, whatever is set here.'}>
      {builtin.map(row)}
      {(sites.data ?? []).map(r => (
        <Row key={r.tool + r.host} icon={<Icon name="globe" size={15} />} title={r.host}
          desc={`${r.tool === 'web_fetch' ? 'Read pages' : 'Open in the browser'} · ${r.action === 'allow' ? 'always allowed' : r.action}`}>
          <button className="btn btn-sm btn-ghost btn-danger" onClick={() => removeSite(r.tool, r.host)}>Remove</button>
        </Row>
      ))}
      {[...byServer].map(([server, rs]) => {
        const changed = rs.filter(r => r.action !== r.default).length
        return (
          <Fragment key={server}>
            <button type="button" className="row" style={{ width: '100%', border: 0, background: 'none', textAlign: 'left', borderTop: '0.5px solid var(--line)' }} onClick={() => setOpenServer(o => o === server ? null : server)} aria-expanded={openServer === server}>
              <span className="row-icon"><Icon name={server === 'browser' ? 'browser' : 'plug'} size={15} /></span>
              <div className="row-main"><div className="row-title">{server === 'browser' ? 'Browser actions' : `${server} tools`}</div>
                <div className="row-desc">{rs.length} tools{changed ? ` · ${changed} changed` : ''}</div></div>
              <Icon name={openServer === server ? 'down' : 'chevron'} size={14} />
            </button>
            {openServer === server && rs.map(row)}
          </Fragment>
        )
      })}
    </Group>
  )
}
