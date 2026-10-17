import { createContext, Fragment, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { copyText, inDesktop, invoke, platform } from '../api'
import { moodOf, type Mood } from '../face'
import { ago, ahead, dayLabel, fullTime, IDLE, pigment, schedule } from '../format'
import { Markdown, SentFile, splitSent } from '../markdown'
import { PinnedCards } from './cards'
import { ask, tell, Avatar, Empty, Icon, talkLabel, useHoldToTalk } from '../ui'
import { failure } from './inbox'
import { Editor } from './memory'
import { useLoad, Bots, skillSlug, when, INCOGNITO, LOCAL, CHANNEL } from '../shared'
import type { Kind, Run, Skill, Routine, Chat, Bot, Busy } from '../types'

type Action = { tool: string; summary: string; decision: string; ok: boolean; result?: string }

/** The note under a reply that promised a reminder and set none (receipts.rs NO_REMINDER): it gets a Set it button. */
const NO_REMINDER = "I said I'd remind you, but no reminder was set."

type UsedMemory = { id: number; kind: Kind; title: string; body: string; status: 'active' | 'superseded' | 'forgotten' }

type Used = { memories: UsedMemory[]; skills: [number, string][]; pages: string[] }

type Message = { id: number; bot_id: number; role: string; content: string; ts: number; actions?: Action[]; run_id?: number; used?: Used }

/** What a reply did, in a few words: ["memory", "Saved to memory · Dentist"]. Lookups and listings aren't worth a line. */
function actionLine(a: Action): [string, string] | null {
  const s = a.summary
  const q = (re: RegExp) => s.match(re)?.[1] ?? ''
  const line: [string, string] | null =
    a.tool === 'memory_store' ? ['memory', `${a.result?.startsWith('Not saved') ? 'Suggested for memory' : 'Saved to memory'} · ${q(/^Remember "(.*?)":/)}`]
    : a.tool === 'memory_forget' ? ['trash', `Forgot · ${q(/"(.*)"$/)}`]
    : a.tool === 'routine_create' ? ['clock', s.startsWith('Once at') ? `Reminder · ${q(/^Once at (.*?): /)}` : s.startsWith('Follow-up at') ? `Follow-up · ${q(/^Follow-up at (.*?): /)}` : `Scheduled · ${schedule(q(/^Create routine \[(.*?)\]/))}`]
    : a.tool === 'routine_delete' ? ['clock', 'Deleted a routine']
    : a.tool === 'ask_bot' ? ['user', `Asked ${q(/^Ask (.*?):/)}`]
    : a.tool === 'web_fetch' ? ['globe', `Read ${q(/^Read https?:\/\/([^/\s]+)/) || 'a page'}`]
    : a.tool === 'web_search' ? ['search', `Searched · ${q(/^Search the web: (.*)$/)}`]
    : a.tool === 'x_search' ? ['search', `Searched X · ${q(/^Search X: (.*)$/)}`]
    : a.tool === 'email_send' ? ['send', a.result?.startsWith('Not done yet') ? `Waiting for your OK · email to ${q(/^Send an email to (.*?): "/)}` : `Sent an email to ${q(/^Send an email to (.*?): "/)}`]
    : a.tool === 'calendar_create' ? ['clock', a.result?.startsWith('Not done yet') ? 'Waiting for your OK · calendar' : `Added · ${q(/^Add to .*?: (.*?".*?")/) || 'to your calendar'}`]
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
  const [skillOpen, setSkillOpen] = useState<number | null>(null)
  const [undone, setUndone] = useState<number[]>([])
  const lines = actions.map(a => [a, actionLine(a)] as const).filter((l): l is readonly [Action, [string, string]] => !!l[1])
  if (!lines.length) return null
  const shown = lines.slice(0, 4)
  // A routine the bot just scheduled: the chip opens the routine as saved, to check, run or undo it.
  const routineOf = (a: Action) => a.tool === 'routine_create' && a.ok ? Number(a.result?.match(/#(\d+)/)?.[1]) || null : null
  // A skill the bot proposed because you asked: the chip opens it, to Save or not.
  const skillOf = (a: Action) => a.tool === 'skill_save' && a.ok ? Number(a.result?.match(/skill #(\d+)/)?.[1]) || null : null
  // A memory the bot saved: Undo right on the chip (the same as Undo in Memory → Learned this week).
  const memOf = (a: Action) => a.tool === 'memory_store' && a.ok ? Number(a.result?.match(/^stored #(\d+)/)?.[1]) || null : null
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

const count = (n: number, one: string, many = one + 's') => `${n} ${n === 1 ? one : many}`

/** What a reply drew on, as one quiet line; open, each memory can be corrected or forgotten right here. */
function UsedLine({ used, chatId, onChange }: { used: Used; chatId: number; onChange: () => void }) {
  const [open, setOpen] = useState(false)
  const [editing, setEditing] = useState<number | null>(null)
  const act = (p: Promise<unknown>) => p.then(onChange, e => tell(String(e)))
  const parts = [
    used.memories.length && count(used.memories.length, 'memory', 'memories'),
    used.skills.length && count(used.skills.length, 'skill'),
    used.pages.length && `read ${count(used.pages.length, 'page')}`,
  ].filter(Boolean)
  return (
    <>
      <button type="button" className={'msg-used' + (open ? ' on' : '')} aria-expanded={open} onClick={() => setOpen(o => !o)}>
        {parts.join(' · ')}<Icon name="chevron" size={10} className="chev" />
      </button>
      {open && (
        <div className="routine-card used-card">
          {used.memories.map(m => (
            <div key={m.id} className={'used-mem' + (m.status === 'active' ? '' : ' gone')}>
              <div className="used-head">
                <span className="mem-title">{m.title}</span>
                {m.status === 'active' ? editing !== m.id && (
                  <span className="used-acts">
                    <button className="btn btn-ghost btn-sm" onClick={() => setEditing(m.id)}>Edit</button>
                    <button className="btn btn-ghost btn-danger btn-sm" onClick={() => act(invoke('forget', { chatId, id: m.id }))}>Forget</button>
                  </span>
                ) : <small>{m.status === 'forgotten' ? 'forgotten since' : 'corrected since'}</small>}
              </div>
              {editing === m.id
                ? <Editor initial={m} submit="Save correction" onCancel={() => setEditing(null)}
                    onSave={(kind, title, body) => act(invoke('remember', { chatId, kind, title, body, supersedes: m.id })).then(() => setEditing(null))} />
                : <p className="mem-body">{m.body}</p>}
            </div>
          ))}
          {used.skills.map(([id, name]) => <div key={'s' + id} className="used-row"><Icon name="bolt" size={12} />Skill · {name}</div>)}
          {used.pages.map((p, i) => <div key={'p' + i} className="used-row"><Icon name="globe" size={12} />{p}</div>)}
        </div>
      )}
    </>
  )
}

/** What you wrote. With a file, the bot got its text; here it's your words and the file's name. */
function Said({ text }: { text: string }) {
  // A reply to one of the bot's messages (Telegram's swipe-to-reply): the quote above what they wrote.
  const re = text.match(/^<replying_to_your_message>([\s\S]*?)<\/replying_to_your_message>\n/)
  if (re) return <><span className="msg-quote">{re[1]}</span><Said text={text.slice(re[0].length)} /></>
  const doc = text.match(/^([\s\S]*?)\n\n\[Attached: (.+?)(?:\. Its text follows|, a scanned document:)/)
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
        {run.reply ? <div className="msg-body selectable"><Markdown text={(r => r.length > 1500 ? r.slice(0, 1500) + '…' : r)(splitSent(run.reply)[0])} /></div>
          : run.error ? <p className="error" style={{ margin: 0 }}>{run.error}</p> : null}
      </div>
    )
  })}</>
}

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

function RoutineCard({ id, chatId, onChange }: { id: number; chatId: number; onChange: () => void }) {
  const rows = useLoad(() => invoke<Routine[]>('routines', { chatId }), [chatId], 10000)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const r = rows.data?.find(x => x.id === id)
  const held = useContext(Paused)(r?.bot_id ?? 0)
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
          <div className="rt">{when(r)}{r.quiet && <span className="badge pending">Quiet</span>}{(r.paused || held !== undefined) && <span className="badge">Paused</span>}{r.delivery === 'digest' && <span className="badge">In digest</span>}{r.delivery === 'log' && <span className="badge">Log only</span>}{r.delivery === 'card' && <span className="badge">On a card</span>}</div>
          <div className="rp selectable">{r.prompt}</div>
          <div className="rm">{r.paused ? 'Paused' : held !== undefined ? `${pausedText(held)} with its bot` : r.next_fire >= IDLE ? 'Runs when a matching event is on the calendar' : `Next run ${ahead(r.next_fire)}`} · change it any time in Routines</div>
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

/** When a bot's pause ends, if it's paused (its own or everyone's; 0: until you resume). */
export const Paused = createContext<(bot: number) => number | undefined>(() => undefined)

export const pausedText = (until: number) => until ? `Paused · back ${ahead(until)}` : 'Paused'

/** Pause choices: until you resume, an hour, tomorrow 8:00, Monday 8:00. */
export function pauseChoices(pause: (until: number) => void): MenuItem[] {
  const at8 = (days: number) => { const d = new Date(); d.setDate(d.getDate() + days); d.setHours(8, 0, 0, 0); return Math.floor(d.getTime() / 1000) }
  const monday = (8 - new Date().getDay()) % 7 || 7
  return [
    ['pause', 'Until I resume', () => pause(0)],
    ['clock', 'For an hour', () => pause(Math.floor(Date.now() / 1000) + 3600)],
    ['clock', 'Until tomorrow 8:00', () => pause(at8(1))],
    ...(monday > 1 ? [['clock', 'Until Monday 8:00', () => pause(at8(monday))] as MenuItem] : []),
  ]
}

type MenuItem = [icon: string, label: string, run: () => void]

/** A small menu at a point (a bot's right-click menu in the sidebar, pause choices). Escape, a click elsewhere or
 * leaving the menu closes it; arrow keys move between its items. */
export function Menu({ at, name, items, onClose }: { at: { x: number; y: number }; name: string; items: MenuItem[]; onClose: () => void }) {
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
      {items.map(([icon, label, run]) => <button key={label} role="menuitem" onClick={run}><Icon name={icon} size={14} />{label}</button>)}
    </div>
  )
}

export function ChatView({ chat, bots, onTurn, incognito = false, setIncognito, openBot, openSettings, botId: chosen, onBot, troubleOf: trouble, only = false, busy: running = [] }: {
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
  // The newest 200, or, once you've paged back (or jumped to an old search hit), everything from `from` on. A ref, not a
  // dep: a changed dep would blank the thread while it reloads.
  const from = useRef<number | null>(null)
  const log = useLoad(() => incognito ? Promise.resolve([] as Message[]) : invoke<Message[]>('messages', { chatId: chat.chat_id, botId: mine, fromId: from.current ?? undefined }), [chat.chat_id, incognito, mine], incognito ? 0 : 4000)
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
  // What you sent while the reply was being written: it changes course to take them in (steering).
  const [steered, setSteered] = useState<string[]>([])
  useEffect(() => { if (!working) setSteered([]) }, [working])
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
  const talk = useHoldToTalk(w => { setText(t => t.trim() ? `${t.trimEnd()} ${w}` : w); box.current?.focus() }, e => setErr(`Couldn't dictate: ${e}`))
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
  const [lit, setLit] = useState<number | null>(null)
  const anim = (m: Message) => (m.id > animFrom ? ' msg-in' : m.id > chipsFrom ? ' acts-in' : '') + (m.id === lit ? ' msg-hit' : '')

  // Earlier messages: null until you ask (a full first page means there may be more).
  const [earlier, setEarlier] = useState<boolean | null>(null)
  const hasEarlier = !incognito && (earlier ?? (log.data?.length ?? 0) >= 200)
  // How far the bottom was, so messages going in above don't move what you're looking at.
  const keep = useRef<number | null>(null)
  const showEarlier = () => {
    if (!msgs[0]) return
    invoke<Message[]>('messages', { chatId: chat.chat_id, botId: mine, beforeId: msgs[0].id }).then(page => {
      setEarlier(page.length >= 200)
      if (!page.length) return
      const el = scroller.current
      if (el) keep.current = el.scrollHeight - el.scrollTop
      from.current = page[0].id
      return log.reload()
    }, e => setErr(String(e)))
  }
  useLayoutEffect(() => {
    const el = scroller.current
    if (keep.current === null || !el) return
    el.scrollTop = el.scrollHeight - keep.current
    keep.current = null
  }, [msgs[0]?.id]) // eslint-disable-line react-hooks/exhaustive-deps

  // Search this chat: a hit loads what's needed and scrolls to it, lit for a moment.
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<{ id: number; ts: number; bot_id: number; role: string; excerpt: string }[] | null>(null)
  useEffect(() => {
    const q = query.trim()
    if (!q || incognito) { setHits(null); return }
    const t = setTimeout(() => invoke<typeof hits>('chat_search', { chatId: chat.chat_id, botId: mine, query: q }).then(setHits, e => setErr(String(e))), 200)
    return () => clearTimeout(t)
  }, [query, chat.chat_id, mine, incognito])
  const target = useRef<number | null>(null)
  const jump = (id: number) => {
    setQuery('')
    target.current = id
    setLit(id)
    setTimeout(() => setLit(l => (l === id ? null : l)), 2400)
    if (msgs[0] && id >= msgs[0].id) return goTo()
    from.current = id
    setEarlier(true)
    log.reload()
  }
  const goTo = () => {
    const el = target.current !== null && document.getElementById(`m${target.current}`)
    if (!el) return
    target.current = null
    atBottom.current = false
    el.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }
  useLayoutEffect(goTo, [log.data]) // eslint-disable-line react-hooks/exhaustive-deps

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
    // The next look is 150 ms after the last answer, so a slow server doesn't get them stacked up.
    let t = 0, off = false
    const poll = () => invoke<{ draft: string | null; step: string | null; done: { summary: string; ok: boolean; decision: string }[] }>('live', { botId: replier, chatId: cid })
      // done is a new array every time: set it only when it says something new.
      .then(l => { setDraft(l?.draft ?? ''); setStep(l?.step ?? null); const d = l?.done ?? []; setDone(o => JSON.stringify(o) === JSON.stringify(d) ? o : d) }, () => {})
      .finally(() => { if (!off) t = window.setTimeout(poll, 150) })
    t = window.setTimeout(poll, 150)
    return () => { off = true; clearTimeout(t); setDraft(''); setStep(null); setDone([]) }
  }, [working, replier, cid])
  useLayoutEffect(() => {
    const t = box.current
    if (t) { t.style.height = 'auto'; t.style.height = `${Math.min(t.scrollHeight, 160)}px` }
  }, [text])

  // Photos pasted or dropped into the composer, shown as thumbnails until sent.
  const [photos, setPhotos] = useState<{ url: string; data: string; ext: string }[]>([])
  // A PDF, Word, Excel or text file to send with the message (one at a time); the bot gets its text.
  const [doc, setDoc] = useState<{ name: string; data: string } | null>(null)
  // Text only, and not incognito (nothing there to take it in): a photo or file waits until the reply is done.
  const steerable = working && !incognito && !photos.length && !doc
  const [dragging, setDragging] = useState(false)
  const addPhotos = (files: Iterable<File>) => {
    for (const f of files) {
      const ext = ({ 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' } as Record<string, string>)[f.type]
      if (!ext) {
        if (f.type === 'application/pdf' || f.type.startsWith('text/') || /\.(pdf|docx|xlsx|txt|md|csv|json|log)$/i.test(f.name)) {
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
    if (working) {
      const t = value.trim()
      if (!t || !steerable) return
      setText('')
      // Not taken (it was just finishing): back in the box, to send once it's done.
      invoke<boolean>('steer', { botId: replier, chatId: cid, text: t }).then(took => took ? setSteered(s => [...s, t]) : setText(x => x || t), e => { setText(x => x || t); setErr(String(e)) })
      return
    }
    const at = incognito ? null : mentioned(value.trim())
    const t = at ? at[1].trim() : value.trim()
    if (!t && !photos.length && !doc) return
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
    // A scanned PDF comes back as its pages, saved as photos to go with the message.
    const read = file ? await invoke<string | { text: string; images: string[] }>('read_document', { botId: to, chatId: LOCAL, name: file.name, data: file.data, text: said }).catch(e => { setErr(String(e)); setFailed(t); return null }) : said
    if (read === null) { setPending(null); return }
    const [message, pages] = typeof read === 'string' ? [read, []] : [read.text, read.images]
    invoke<string>('send_local', { botId: to, text: message, images: [...images, ...pages] })
      .then(() => flash('happy'), e => { if (/stopped/i.test(String(e))) flash('surprised', 900); else { setErr(String(e)); setFailed(t) } })
      // Swap the pending bubbles for the saved messages in one frame: no flash in between.
      .finally(() => { ownSend.current = true; log.reload().finally(() => { setPending(null); setTo(null) }); conv.reload(); onTurn(); if (only && to !== botId) onBot?.(to) })
  }
  const bubble = (m: Message, last = false) => {
    const sched = m.role === 'user' ? m.content.match(SCHEDULED) : null
    if (sched) return (
      <div id={`m${m.id}`} className={'msg-trigger' + anim(m)} title={sched[4]}>
        <Icon name={sched[1] === 'quiet check' ? 'moon' : 'clock'} size={13} />
        <span>{sched[1] === 'quiet check' ? 'Quiet check' : 'Routine'} · {schedule(sched[3])} — {sched[4]}</span>
      </div>
    )
    if (m.role === 'user') return <div id={`m${m.id}`} className={'msg-user' + anim(m)} title={fullTime(m.ts)}><Said text={m.content} /></div>
    const b = botOf(m.bot_id)
    const [text, files] = splitSent(m.content)
    return (
      <div id={`m${m.id}`} className={'msg-bot' + anim(m)} style={{ '--pig': pigment(b.id) } as React.CSSProperties}>
        <Avatar id={b.id} name={b.name} size={26} still />
        <div>
          <div className="msg-head"><b>{b.name}</b><time title={fullTime(m.ts)}>{ago(m.ts)}</time><CopyButton text={text} /></div>
          {text && <div className="msg-body"><Markdown text={text} /></div>}
          {files.map(f => <SentFile key={f} chatId={chat.chat_id} name={f} />)}
          {/* Only under the latest message: once anything follows, it's been answered one way or another. */}
          {m.content.includes(NO_REMINDER) && m === msgs[msgs.length - 1] && <button type="button" className="btn btn-sm" onClick={() => send(`@${b.name} Set the reminder you said you would.`)} disabled={working}>Set it</button>}
          {m.actions && <Actions actions={m.actions} chatId={chat.chat_id} runId={m.run_id} onChange={() => { log.reload(); onTurn() }} />}
          {m.used && <UsedLine used={m.used} chatId={chat.chat_id} onChange={() => log.reload()} />}
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
        {!incognito && <PinnedCards chatId={chat.chat_id} botId={mine} botName={id => botOf(id).name} />}
        {!incognito && (msgs.length > 0 || hasEarlier) && (
          <div className="chat-search">
            <label className="search"><Icon name="search" size={14} />
              <input className="input" type="search" placeholder="Search this chat" aria-label="Search this chat" value={query}
                onChange={e => setQuery(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') setQuery('') }} /></label>
            {hits && query.trim() && (
              <div className="transcript">{hits.length ? hits.map(h => (
                <button key={h.id} type="button" className="tl hit" onClick={() => jump(h.id)} title={fullTime(h.ts)}>
                  <span className="mono">{ago(h.ts)}</span><b>{h.role === 'user' ? 'You' : botOf(h.bot_id).name}</b><span>{h.excerpt}</span>
                </button>
              )) : <span className="hint">Nothing in this chat matches.</span>}</div>
            )}
          </div>
        )}
        <div className="thread selectable">
          {hasEarlier && msgs.length > 0 && <button type="button" className="btn btn-sm btn-ghost show-earlier" onClick={showEarlier}>Show earlier messages</button>}
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
              {steered.map((t, i) => <div key={i} className="msg-user msg-in"><Said text={t} /></div>)}
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
                    mood={working ? moodOf(step, draft) : react ?? (talk.state !== 'idle' || text.trim() ? 'listening' : err ? 'sad' : 'idle')} />
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
                <textarea ref={box} rows={1} placeholder={incognito ? `Message ${me.name} incognito` : working ? 'Add to what you asked, or change it' : `Message ${me.name}`} value={text} onChange={e => setText(e.target.value)} disabled={working && !steerable} autoFocus
                  onPaste={e => { if (incognito) return; const imgs = [...e.clipboardData.files].filter(f => f.type.startsWith('image/')); if (imgs.length) { e.preventDefault(); addPhotos(imgs) } }}
                  onKeyUp={talk.keys.onKeyUp} onBlur={talk.keys.onBlur}
                  onKeyDown={e => {
                    talk.keys.onKeyDown(e)
                    if (e.key === 'Tab' && skillHits[0]) { e.preventDefault(); setText(`/${skillSlug(skillHits[0].name)} `); return }
                    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); send() }
                  }} />
                {!working && setIncognito && (
                  <button type="button" className={'incog-toggle' + (incognito ? ' on' : '')} aria-pressed={incognito} onClick={() => setIncognito(!incognito)}
                    title={incognito ? 'Back to the normal chat' : 'Incognito chat: no memory, nothing saved'}><Icon name="incognito" size={16} /></button>
                )}
                {working && steerable && text.trim() && <button className="send" aria-label="Send now: the reply changes course" title="Send now: the reply changes course"><Icon name="send" size={15} /></button>}
                {working
                  ? <button type="button" className="send stop" aria-label="Stop" title="Stop" onClick={() => invoke('stop', { botId: replier, chatId: cid }).catch(e => setErr(`Couldn't stop it: ${e}`))}><span className="stop-square" /></button>
                  : <button className="send" aria-label="Send" disabled={!text.trim() && !photos.length}><Icon name="send" size={15} /></button>}
              </form>
              <p className="composer-note">{talk.state === 'listening' ? `Listening… let go of ${talkLabel} to stop` : talk.state === 'writing' ? 'Writing it out…'
                : (platform === 'mac' ? 'Return to send · Shift-Return for a new line' : 'Enter to send · Shift+Enter for a new line') + (inDesktop ? ` · Hold ${talkLabel} to talk` : '')}</p>
            </>
          ) : (
            <p className="composer-note">Reply from {CHANNEL[chat.channel]?.name ?? 'the app'}; this is a read-only view.</p>
          )}
        </div>
      </div>
    </div>
  )
}
