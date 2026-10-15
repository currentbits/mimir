import { useContext, useEffect, useState } from 'react'
import { invoke } from '../api'
import { ago, ahead, fullTime } from '../format'
import { Markdown, openLink } from '../markdown'
import { ask, tell, Avatar, Empty, Icon, Row } from '../ui'
import { RUN_TITLE } from './activity'
import { toolLabel } from './settings'
import { Bots, INCOGNITO } from '../shared'
import type { Approval, Chat, Hold, Inbox, Run, Tab } from '../types'

/** What an approval would do, in a few words, and whether it can become an "always allow". */
function approvalRisk(a: Approval): { icon: string; label: string; tone: string; always: boolean } {
  if (a.kind === 'signin') return { icon: 'key', label: 'Sign in in Mimir, never in a chat: Mimir types it into the page and keeps it for this bot on this site only. The bot never sees it.', tone: 'login', always: false }
  if (a.kind === 'takeover') return { icon: 'laptop', label: 'A step only you should do. Take over on the live screen, then press Give back; the bot carries on.', tone: 'login', always: false }
  if (a.kind === 'login') return { icon: 'key', label: 'Types a saved login. Every use asks you.', tone: 'login', always: false }
  if (a.kind === 'send' && a.tool === 'send_file') return { icon: 'send', label: 'Sends a file where others see it (a group, or a guest\'s chat). Every file there asks you.', tone: 'rule', always: false }
  if (a.kind === 'send') return { icon: 'send', label: 'Sends an email as you, to the address shown first. Every email asks you.', tone: 'rule', always: false }
  if (a.kind === 'remind') return { icon: 'user', label: 'Sends to someone else’s chat in your household. Asked the first time for each person.', tone: 'rule', always: false }
  if (a.kind === 'bot') return { icon: 'user', label: 'Makes a new bot with this role, on the same AI account. Every new bot asks you.', tone: 'rule', always: false }
  if (a.kind === 'plan') return { icon: 'check', label: 'One OK for the whole plan: these steps won\'t ask again during this reply.', tone: 'rule', always: false }
  if (a.kind === 'unasked') return { icon: 'shield', label: 'Asking because this doesn’t look like what you asked for (who or what it acts on wasn’t in your message).', tone: 'guarded', always: false }
  if (a.kind === 'loop') return { icon: 'retry', label: 'The bot keeps making this same call. Allow lets it carry on; Deny tells it to stop and say where it got stuck.', tone: 'rule', always: false }
  if (a.kind === 'guest') return { icon: 'user', label: 'Asked in a guest\'s chat. An OK there is for this once; your rules stay yours.', tone: 'rule', always: false }
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

/** Let an item play its exit before the action that removes it: `leave(id, run)`, `leaving.has(id)`. */
export function useLeave() {
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
        <div className="ask">{b.name} {a.kind === 'takeover' ? 'needs you on the computer' : a.kind === 'signin' ? 'needs you to sign in' : 'needs your OK'} · {where}</div>
        <div className={'what selectable' + (a.kind === 'plan' ? ' plan' : '')}>{a.summary}</div>
        <div className="risk"><Icon name={risk.icon} size={12} />{risk.label}</div>
        {learned && <div className="learned">You've allowed this {a.allowed_before} times. Stop asking?</div>}
        {a.parked && <div className="parked">From a routine. It runs when you approve; it waits {((w: string) => w.startsWith('in ') ? `for ${w.slice(3)}` : `until ${w}`)(ahead(a.expires_at))}.</div>}
      </div>
      {a.kind === 'signin' ? <div className="choices">
        <button className="btn" onClick={() => decide(a.id, false)}>Can’t now</button>
        <a className="btn btn-primary" href="#signin">Sign in</a>
      </div> : a.kind === 'takeover' ? <div className="choices">
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
export function Approvals({ list, chats, chatName, onDecided }: { list: Approval[]; chats: Chat[]; chatName: (c: Chat) => string; onDecided: () => void }) {
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

/** The masked form a sign-in card links to (#signin): for the newest card waiting. Only that card's id goes back with
 *  what you type; the link itself carries nothing (no server, token or site), so a link someone sends you can't
 *  point it elsewhere. */
export function SignInForm({ list, onClose }: { list: Approval[]; onClose: () => void }) {
  const botOf = useContext(Bots)
  // Kept once sent: the card leaves the list as soon as Mimir starts typing.
  const [held, setHeld] = useState<Approval>()
  const a = held ?? list.filter(x => x.kind === 'signin').at(-1)
  const [f, setF] = useState({ user: '', password: '', code: '', totp: '' })
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [done, setDone] = useState<boolean | null>(null)
  const field = (k: keyof typeof f) => ({ value: f[k], onChange: (e: { target: { value: string } }) => setF({ ...f, [k]: e.target.value }) })
  const send = () => {
    if (!a) return
    setBusy(true); setErr(null); setHeld(a)
    invoke<{ signed_in: boolean }>('signin_submit', { id: a.id, ...f })
      .then(r => { setF({ user: '', password: '', code: '', totp: '' }); setDone(r.signed_in) }, e => { setErr(String(e)); setHeld(undefined) }).finally(() => setBusy(false))
  }
  return (
    <div className="ask-backdrop" onMouseDown={e => e.target === e.currentTarget && !busy && onClose()}>
      <form className="ask-box" role="dialog" aria-modal="true" aria-label="Sign in" onSubmit={e => { e.preventDefault(); send() }} onKeyDown={e => e.key === 'Escape' && !busy && onClose()}>
        {done !== null ? <>
          <p>{done ? `Signed in to ${a?.site ?? 'the site'}. ${botOf(a?.bot_id ?? 0).name} carries on.` : 'That didn’t sign in. The bot will tell you; it can ask again, or you can take over on the computer.'}</p>
          <div className="ask-actions"><button type="button" className="btn btn-primary" onClick={onClose}>OK</button></div>
        </> : !a ? <>
          <p>{err ? `That didn’t sign in: ${err}` : 'No sign-in is waiting: it was used, or ran out after 10 minutes. Ask the bot again.'}</p>
          <div className="ask-actions"><button type="button" className="btn btn-primary" onClick={onClose}>OK</button></div>
        </> : <>
          <p><b>Sign in to {a.site}</b> for {botOf(a.bot_id).name}. Mimir types this into the page on the computer and keeps it for this bot on {a.site} only. The bot never sees it.</p>
          <input className="input" aria-label="User name or email" placeholder="User name or email" autoComplete="username" autoFocus {...field('user')} />
          <input className="input" aria-label="Password" placeholder="Password" type="password" autoComplete="current-password" {...field('password')} />
          <input className="input" aria-label="Code (if the site asks for one)" placeholder="Code, if the site asks for one" inputMode="numeric" autoComplete="one-time-code" {...field('code')} />
          <input className="input" aria-label="Authenticator setup key (optional)" placeholder="Authenticator setup key or otpauth:// link (optional)" type="password" autoComplete="off" {...field('totp')} />
          {err && <p className="error">{err}</p>}
          <div className="ask-actions">
            <button type="button" className="btn btn-ghost" disabled={busy} onClick={onClose}>Cancel</button>
            <button className="btn btn-primary" disabled={busy || !(f.user || f.password || f.code || f.totp)}>{busy ? 'Signing in…' : 'Sign in'}</button>
          </div>
        </>}
      </form>
    </div>
  )
}

/** One item per usage limit: every bot on that AI login waits for the same reset. */
const holdGroups = (hs: Hold[]) => Object.values(hs.reduce<Record<string, Hold[]>>((g, h) => { const k = h.cap || h.routine_id ? h.key : `limit:${h.until}`; (g[k] ??= []).push(h); return g }, {}))

export const inboxCount = (i: Inbox | null) => i ? (i.server_update ? 1 : 0) + (i.damage ? 1 : 0) + i.approvals.length + i.missed.length + i.failed.length + holdGroups(i.holds ?? []).length + i.suggestions.reduce((n, s) => n + s.count, 0) + (i.unconfirmed ?? []).reduce((n, s) => n + s.count, 0) : 0

/** A failed run in plain words, with the fix that most likely helps. */
export function failure(error: string | null, status?: Run['status'], delivered?: string | null): { title: string; fix: 'bot' | 'chat' | 'routines' | null; fixLabel?: string } {
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

export function InboxView({ data, troubled, reload, chats, chatName, openBot, openChat, openSettings }: {
  data: Inbox | null; troubled: { id: number; text: string }[]; reload: () => unknown; chats: Chat[]; chatName: (c: Chat) => string; openBot: (id: number) => void; openChat: (chat: number, view: Tab) => void; openSettings: () => void
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
      {data.damage && <>
        <div className="section-title" style={{ marginTop: 0 }}><h2>Your data</h2></div>
        <div className="group">
          <div className="inbox-row">
            <span className="row-icon"><Icon name="alert" size={15} /></span>
            <div className="inbox-main">
              <div className="inbox-title">The database reports damage</div>
              <div className="inbox-desc selectable">Today's check found: {data.damage}. No backup was taken over the good ones; restore the latest from Settings.</div>
            </div>
            <div className="row-ctl"><button className="btn btn-sm" onClick={openSettings}>Restore a backup…</button></div>
          </div>
        </div>
      </>}
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
