import { Fragment, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { inDesktop, insecureServer, invoke, saveDownload, localInvoke, probe, remoteServer, setWebToken, connectServer, shortcut, trayName } from '../api'
import { ago, fullTime } from '../format'
import { openLink } from '../markdown'
import { ask, tell, Empty, Group, Icon, Row, Segmented, Switch } from '../ui'
import { SignInChatGPT, SignInGrok, ConnectOpenRouter, SignInBrowser } from './accounts'
import { useLoad, Bots, SECRETS_AT, LOCAL } from '../shared'
import type { Bot, ComputerStatus, Chat, Account, ToolServer } from '../types'
import { DIRECTORY, type Listed } from '../directory'

type VaultEntry = { name: string; domains: string[]; bot?: number; totp?: boolean }

export type Rule = { tool: string; description: string; action: 'always' | 'allow' | 'ask' | 'deny'; default: string }

/** Words a setting goes by besides its own text, so a search finds it ("token" → AI keys, "2fa"…). */
const SETTING_WORDS: Record<string, string> = {
  'AI keys': 'api key token openai openrouter xai grok anthropic model provider chatgpt sign in plan',
  'Email': 'mail imap smtp gmail icloud fastmail inbox app password send draft',
  'Calendar': 'caldav icloud fastmail nextcloud events schedule',
  'Web search': 'brave searxng google internet search engine x twitter tweets posts bearer token',
  'Scanned PDFs': 'ocr scan local model ollama dots.ocr vision photos pages private',
  'Daily digest': 'quiet hours notifications summary weekly review do not disturb',
  'Limits': 'cap budget tokens cost usage limit plan subscription quota five hour weekly cheaper model haiku',
  'Permissions': 'allow ask deny approve rules tools safety always',
  'Main bot': 'main primary first contact coordinator team bots tidy',
  'Export folders': 'export copy files folder skills file_export',
  'Ask first rules': 'email domain outside recipient folder ask deny rule target',
  'Browser': 'playwright chromium saved logins passwords vault',
  'Tool servers': `mcp plugins integrations connectors apps browse directory ${DIRECTORY.map(d => d.name).join(' ')}`,
  'Sensitive sources': 'bank banking health medical private finance money leak',
  'Your data': 'export backup download delete privacy doctor check health troubleshoot problem usage stats telemetry analytics',
  'Chat API': 'openai token shortcuts siri home assistant integration v1 completions',
  'Webhooks out': 'home assistant ntfy n8n zapier notify notification push phone light hmac signed events integration',
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

export function SettingsView({ onComputer, find = '' }: { onComputer: () => void; find?: string }) {
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
      <MainBotSettings />
      <DecisionsSettings />
      <LanguageSettings />
      <LimitsSettings />
      <ApiKeySettings />
      <WebSearchSettings />
      <ScanSettings />
      <EmailSettings />
      <CalendarSettings />
      <KnowMeSettings />
      <MemoryFolderSettings />
      <ComputerSettings onChange={() => { onComputer(); toolsChanged() }} />
      <BrowserSettings onChange={toolsChanged} />
      <ToolServerSettings onChange={toolsChanged} />
      <SensitiveSettings key={'s' + tools} />
      <Permissions key={tools} />
      <AskFirstRules />
      <ExportFolders />
      <WebhooksOutSettings />
      <DataSettings />
      {(!inDesktop || remoteServer()) && <ChatApiSettings />}
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
      <BackupRows />
      <UsageStatsRow />
      <DoctorRow />
    </Group>
  )
}

/** The daily anonymous count to us (usage_stats.rs), with exactly what it sends. */
function UsageStatsRow() {
  const st = useLoad(() => invoke<{ on: boolean; sample: unknown[] }>('usage_stats', {}), [], 60000)
  const [show, setShow] = useState(false)
  const s = st.data
  if (!s) return null
  return <>
    <Row icon={<Icon name="activity" size={15} />} title="Send anonymous usage stats"
      desc={<>Once a day: a random id for this install, the version and OS, each bot’s AI provider and model, and on a server how many pages its browser opened and how many a site blocked. Never names, messages, memories or keys. <button type="button" className="linkish" onClick={() => setShow(!show)}>{show ? 'Hide' : 'See what’s sent'}</button></>}>
      <Switch label="Send anonymous usage stats" checked={s.on} onChange={on => invoke('usage_stats', { on }).then(st.reload, e => tell(String(e)))} />
    </Row>
    {show && <div className="row-block"><pre className="mono" style={{ margin: 0, whiteSpace: 'pre-wrap', fontSize: 12 }}>{JSON.stringify(s.sample, null, 1)}</pre></div>}
  </>
}

type Backups = { dir: string; list: { name: string; size: number; taken_at: number }[] }

/** The daily copies of the database, and putting one back. */
function BackupRows() {
  const b = useLoad(() => invoke<Backups>('backups'), [], 60000)
  const [pick, setPick] = useState('')
  const [msg, setMsg] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  if (!b.data) return null
  const list = b.data.list
  const chosen = pick || list[0]?.name || ''
  const label = (s: Backups['list'][number]) => `${fullTime(s.taken_at)}${s.name.includes('before-restore') ? ' (before a restore)' : ''} · ${(s.size / 1e6).toFixed(1)} MB`
  const now = () => { setBusy(true); invoke('backup_now').then(() => { setMsg('Backed up just now.'); b.reload() }, e => setMsg(String(e))).finally(() => setBusy(false)) }
  const restore = async () => {
    const s = list.find(x => x.name === chosen)
    if (!s || !await ask(`Put back the copy from ${label(s)}? Everything since then (memories, chats, routines) goes back to how it was. A copy of now is kept first, so you can undo this.`, 'Restore', true)) return
    setBusy(true)
    invoke('backup_restore', { name: chosen }).then(() => { setMsg(`Restored the copy from ${fullTime(s.taken_at)}.`); b.reload() }, e => setMsg(String(e))).finally(() => setBusy(false))
  }
  return <>
    <Row icon={<Icon name="clock" size={15} />} title="Daily backup" desc={msg ?? (list.length ? `Last one ${ago(list[0].taken_at)}. Mimir checks the database and keeps a copy every day: the last 7 days and 4 weeks, in ${b.data.dir}.` : `Mimir checks the database and keeps a copy every day: the last 7 days and 4 weeks, in ${b.data.dir}.`)}>
      <button type="button" className="btn btn-sm" disabled={busy} onClick={now}>Back up now</button>
    </Row>
    <Row icon={<Icon name="history" size={15} />} title="Restore a backup…" desc={list.length ? 'Puts a copy back in place of what Mimir has now. To restore one from elsewhere, put it in the backups folder first.' : 'No backups yet.'}>
      {list.length > 0 && <select className="select select-inline" aria-label="Which backup" value={chosen} onChange={e => setPick(e.target.value)}>
        {list.map(s => <option key={s.name} value={s.name}>{label(s)}</option>)}
      </select>}
      <button type="button" className="btn btn-sm" disabled={busy || !list.length} onClick={restore}>Restore</button>
    </Row>
  </>
}

type Check = { state: 'ok' | 'warn' | 'fail'; what: string; fix: string }

/** `mimir doctor`: everything Mimir needs, checked once. Nothing is spent or sent. */
function DoctorRow() {
  const [checks, setChecks] = useState<Check[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const run = () => { setBusy(true); invoke<Check[]>('doctor').then(c => { setChecks(c); setErr(null) }, e => setErr(String(e))).finally(() => setBusy(false)) }
  const mark = { ok: '✓', warn: '!', fail: '✗' }
  return <Row icon={<Icon name="check" size={15} />} title="Check everything" desc={err ?? (checks
    ? checks.map((c, i) => <div key={i} className={c.state === 'fail' ? 'error' : undefined}>{mark[c.state]} {c.what}{c.fix && `. ${c.fix}`}</div>)
    : 'AI sign-ins, each bot’s Telegram, Slack and Discord connection, email, the bots’ computer, disk space, the database and its backups. Nothing is spent or sent.')}>
    <button type="button" className="btn btn-sm" disabled={busy} onClick={run}>{busy ? 'Checking…' : 'Check'}</button>
  </Row>
}

type WebhookOut = { url: string | null; events: string[]; text: boolean; secret: string | null }

const HOOK_EVENTS: [string, string][] = [['routine_finished', 'A routine finished'], ['routine_failed', 'A routine failed'], ['approval_waiting', 'An approval is waiting'], ['reply_sent', 'A reply was sent']]

/** A signed JSON to one address when something happens: flash a light, push a phone notification, start an n8n flow. */
function WebhooksOutSettings() {
  const [d, setD] = useState<WebhookOut | null>(null)
  const [msg, setMsg] = useState<ReactNode>(null)
  const fail = (e: unknown) => setMsg(<span className="error">{String(e)}</span>)
  useEffect(() => { invoke<WebhookOut>('webhook_out', {}).then(setD, fail) }, [])
  if (!d) return null
  const save = (v: WebhookOut, rotate = false) => invoke<WebhookOut>('webhook_out', { url: v.url ?? '', events: v.events, text: v.text, rotate }).then(r => { setD(r); setMsg(null) }, fail)
  const test = () => invoke('webhook_out_test').then(() => setMsg('Sent a test. Check that it arrived.'), fail)
  const rotate = async () => { if (await ask('Make a new signing secret? Whatever checks the old one refuses these calls until you give it the new one.', 'New secret', true)) void save(d, true) }
  return (
    <Group title="Webhooks out" note={msg ?? <>When something happens, Mimir posts a small JSON (the event, bot, chat, routine, a short summary and a link to Needs you) to this address, signed in <code>X-Hub-Signature-256</code> the way GitHub signs. A failed send is tried again a few times. Message text is left out unless you include it.</>}>
      <form className="row-block" onSubmit={e => { e.preventDefault(); void save(d) }}>
        <div className="editor" style={{ margin: 0 }}><div className="er">
          <input className="input" type="url" placeholder="https://ntfy.sh/…, or a Home Assistant or n8n webhook" aria-label="Webhook address" value={d.url ?? ''} onChange={e => setD({ ...d, url: e.target.value })} />
          <button className="btn">Save</button>
          {d.url && <button type="button" className="btn btn-ghost" onClick={test}>Send a test</button>}
        </div></div>
      </form>
      {HOOK_EVENTS.map(([id, label]) => (
        <Row key={id} title={label}>
          <Switch label={label} checked={d.events.includes(id)} onChange={on => save({ ...d, events: on ? [...d.events, id] : d.events.filter(e => e !== id) })} />
        </Row>
      ))}
      <Row title="Include the text" desc="The routine's result or error, the reply, or what the approval is for. It leaves Mimir for that address.">
        <Switch label="Include the text" checked={d.text} onChange={text => save({ ...d, text })} />
      </Row>
      {d.secret && <Row icon={<Icon name="key" size={15} />} title="Signing secret" desc={<code className="selectable">{d.secret}</code>}>
        <button type="button" className="btn btn-sm" onClick={() => navigator.clipboard.writeText(d.secret!).catch(() => {})}><Icon name="copy" size={13} />Copy</button>
        <button type="button" className="btn btn-sm btn-ghost" onClick={rotate}>New</button>
      </Row>}
    </Group>
  )
}

/** Server only: tokens for an OpenAI-compatible chat API, each reaching one bot (Shortcuts, Home Assistant). */
function ChatApiSettings() {
  const tokens = useLoad(() => invoke<{ id: string; name: string; bot_id: number }[]>('chat_tokens'), [])
  const bots = useLoad(() => invoke<Bot[]>('bots'), []).data ?? []
  const [bot, setBot] = useState<number | null>(null)
  const [name, setName] = useState('')
  const [made, setMade] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const base = `${remoteServer()?.url ?? location.origin}/v1`
  const botId = bot ?? bots[0]?.id
  const add = () => invoke<string>('chat_token_add', { botId, name }).then(t => { setMade(t); setName(''); setErr(null); tokens.reload() }, e => setErr(String(e)))
  const remove = async (id: string, n: string) => {
    if (await ask(`Delete the token for “${n}”? Whatever uses it stops working.`, 'Delete', true)) invoke('chat_token_delete', { id }).then(tokens.reload, e => setErr(String(e)))
  }
  return (
    <Group title="Chat API" note={err ? <span className="error">{err}</span> : <>For apps that speak OpenAI’s chat API. Base URL <code className="selectable">{base}</code>; each token reaches one bot, in a chat of its own (no email, calendar, meetings or computer there), and nothing else. Approvals still come here.</>}>
      {(tokens.data ?? []).map(t => (
        <Row key={t.id} icon={<Icon name="key" size={15} />} title={t.name} desc={bots.find(b => b.id === t.bot_id)?.name}>
          <button type="button" className="btn btn-ghost btn-danger btn-sm" aria-label={`Delete ${t.name}`} onClick={() => remove(t.id, t.name)}><Icon name="trash" size={13} /></button>
        </Row>
      ))}
      {made && <Row icon={<Icon name="check" size={15} />} title="Copy this token now: it isn’t shown again" desc={<code className="selectable">{made}</code>}>
        <button type="button" className="btn btn-sm" onClick={() => navigator.clipboard.writeText(made).then(() => setMade(null), () => {})}><Icon name="copy" size={13} />Copy</button>
      </Row>}
      <form className="row-block" onSubmit={e => { e.preventDefault(); add() }}>
        <div className="editor" style={{ margin: 0 }}><div className="er">
          <input className="input" placeholder="Used by (Shortcuts, Home Assistant…)" aria-label="Token name" value={name} onChange={e => setName(e.target.value)} />
          <select className="select" style={{ width: 'auto' }} aria-label="Bot" value={botId ?? ''} onChange={e => setBot(Number(e.target.value))}>
            {bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
          </select>
          <button className="btn" disabled={!name.trim() || botId == null}>New token</button>
        </div></div>
      </form>
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

/** The language Mimir itself writes in on Telegram, Slack and Discord (bots answer in yours either way). */
function LanguageSettings() {
  const [l, setL] = useState<{ lang: string; system: string; langs: [string, string][] } | null>(null)
  useEffect(() => { invoke<typeof l>('language').then(setL, () => {}) }, [])
  if (!l) return null
  const name = (code: string) => l.langs.find(([c]) => c === code)?.[1] ?? code
  return (
    <Group title="Language" note="What Mimir itself says in Telegram, Slack and Discord: pairing, approval cards, errors, the digest and the weekly list. Your bots answer in the language you write in either way.">
      <Row icon={<Icon name="globe" size={15} />} title="Mimir writes in">
        <select className="select" style={{ width: 'auto' }} aria-label="Language" value={l.lang}
          onChange={e => invoke<typeof l>('language', { lang: e.target.value }).then(setL, () => {})}>
          <option value="">{`System (${name(l.system)})`}</option>
          {l.langs.map(([c, n]) => <option key={c} value={c}>{n}</option>)}
        </select>
      </Row>
    </Group>
  )
}

/** Desktop only: the app is the bots' home, so it should be running. */
/** When routines in the digest lane deliver their one message of the day. */
function DigestSettings() {
  const [at, setAt] = useState<string | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [weekly, setWeekly] = useState<boolean | null>(null)
  const [quiet, setQuiet] = useState<string | null>(null)
  const [away, setAway] = useState<number | null>(null)
  useEffect(() => {
    invoke<number>('away_days').then(setAway, () => {})
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
        desc={quiet ? 'Routines that would message you wait for your digest, and asks from routines wait until morning. Replies to your own messages still come, and so does a routine whose “Come through right away if” rule matches (up to 3 a day).' : 'Off: routines message you whenever they run.'}>
        {quiet !== null && <>
          {quiet && <>
            <input className="input" type="time" aria-label="Quiet from" style={{ width: 'auto' }} value={qFrom} onChange={e => e.target.value && saveQuiet(`${e.target.value}-${qTo}`)} />
            <span className="hint">to</span>
            <input className="input" type="time" aria-label="Quiet until" style={{ width: 'auto' }} value={qTo} onChange={e => e.target.value && saveQuiet(`${qFrom}-${e.target.value}`)} />
          </>}
          <Switch label="Quiet hours" checked={!!quiet} onChange={v => saveQuiet(v ? '22:00-07:00' : '')} />
        </>}
      </Row>
      <Row icon={<Icon name="pause" size={15} />} title="Pause routines when I'm away"
        desc={away ? `After ${away} days without a message from you to any bot, routines stop until you write again (or press Resume in Needs you), so they don't use up your AI plan on results nobody reads. Bots still answer.` : 'Off: routines keep running however long you are away.'}>
        {away !== null && <>
          {away > 0 && <><input className="input" type="number" min={1} max={365} aria-label="Days away" style={{ width: 72 }} value={away} onChange={e => { const d = Number(e.target.value); if (d >= 1 && d <= 365) { setAway(d); void invoke<number>('away_days', { days: d }).then(setAway, x => setMsg(String(x))) } }} /><span className="hint">days</span></>}
          <Switch label="Pause routines when I'm away" checked={away > 0} onChange={v => { const d = v ? 7 : 0; setAway(d); void invoke<number>('away_days', { days: d }).then(setAway, x => setMsg(String(x))) }} />
        </>}
      </Row>
      <Row icon={<Icon name="memory" size={15} />} title="Weekly “what I learned”" desc="Once a week at that time, each Telegram, Slack or Discord chat gets a list of what your bots learned there on their own. Reply /undo with a number to take one back.">
        {weekly !== null && <Switch label="Weekly what I learned" checked={weekly} onChange={v => { setWeekly(v); invoke<boolean>('weekly_review', { on: v }).then(setWeekly, () => setWeekly(!v)) }} />}
      </Row>
    </Group>
  )
}

/** The bot that's your first point of contact: it sees how the others are doing, and runs the daily memory tidy-up. */
function MainBotSettings() {
  const bots = useLoad(() => invoke<Bot[]>('bots'), [], 60000)
  const main = useLoad(() => invoke<number | null>('main_bot'), [], 60000)
  const [msg, setMsg] = useState<string | null>(null)
  if ((bots.data ?? []).length < 2) return null
  return (
    <Group title="Main bot" note="Your first point of contact when you have several bots: it hands work to the one that fits, tells you when another bot's runs fail or it's waiting for you, and runs the daily memory tidy-up. Ask it to check in every morning if you'd like that.">
      <Row icon={<Icon name="user" size={15} />} title="Main bot" desc={msg ?? 'The others know to point you there for how things are going.'}>
        <select className="select" style={{ width: 'auto' }} aria-label="Main bot" value={main.data ?? ''} onChange={e => invoke<number>('main_bot', { botId: Number(e.target.value) }).then(() => { setMsg(null); main.reload() }, x => setMsg(String(x)))}>
          {(bots.data ?? []).map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
        </select>
      </Row>
    </Group>
  )
}

/** One private chat that also gets every chat's asks from routines, failed routines and memory suggestions. */
function DecisionsSettings() {
  const [d, setD] = useState<{ chat: number | null; options: Chat[] } | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  useEffect(() => { invoke<typeof d>('decisions_chat').then(setD, () => {}) }, [])
  const pick = (chat: number) => invoke<typeof d>('decisions_chat', { chat }).then(v => { setD(v); setMsg(null) }, e => setMsg(String(e)))
  return (
    <Group title="Decisions" note="Asks from routines in any chat, including this app, also come here as a card; answer it in either place and both close. Failed routines and new memory suggestions are mentioned here too. Groups and guests' chats can't be picked.">
      <Row icon={<Icon name="inbox" size={15} />} title="Also send decisions to" desc={msg ?? (d && !d.options.length ? 'Pair a private chat in Telegram, Slack or Discord first.' : 'Only the chat they came from, unless you pick one.')}>
        {d && <select className="select" style={{ width: 'auto' }} value={d.chat ?? 0} aria-label="Decisions chat" onChange={e => pick(Number(e.target.value))}>
          <option value={0}>No other chat</option>
          {d.options.map(c => <option key={c.chat_id} value={c.chat_id}>{c.title} ({c.channel})</option>)}
        </select>}
      </Row>
    </Group>
  )
}

type Caps = { routine: number | null; bot: number | null; day: number | null; person: number | null }

type Windows = { at: number; status: string; windows: { name: string; used: number; resets_at: number }[] }

const WINDOW_NAME: Record<string, string> = { five_hour: '5-hour window', seven_day: 'Weekly', seven_day_opus: 'Weekly (Opus)', seven_day_sonnet: 'Weekly (Sonnet)' }

/** Daily caps on what routines may use, and the AI plan's own windows where the AI reports them. */
const KIND_NAME: Record<string, string> = { claude: 'Claude', codex: 'ChatGPT / Codex', grok: 'Grok', api: 'API key' }

function LimitsSettings() {
  const [caps, setCaps] = useState<Caps | null>(null)
  const [win, setWin] = useState<Windows | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [models, setModels] = useState<{ kind: string; model: string }[]>([])
  const [fresh, setFresh] = useState<number | null>(null)
  useEffect(() => {
    invoke<{ kind: string; model: string }[]>('routine_model').then(setModels, () => {})
    invoke<Caps>('caps').then(setCaps, () => {})
    invoke<Windows | null>('ai_windows').then(setWin, () => {})
    invoke<number>('fresh_after_hours').then(setFresh, () => {})
  }, [])
  const save = (k: keyof Caps, v: string) => invoke<Caps>('caps', { [k]: v.trim() ? Math.max(0, Math.round(Number(v) * 1000)) : 0 }).then(c => { setCaps(c); setMsg(null) }, e => setMsg(String(e)))
  const field = (k: keyof Caps, title: string, desc: string) => (
    <Row icon={<Icon name="activity" size={15} />} title={title} desc={desc}>
      {caps && <><input className="input mono" type="number" min={0} step={10} aria-label={title} placeholder="No cap" style={{ width: 96 }}
        defaultValue={caps[k] ? caps[k]! / 1000 : ''} onBlur={e => save(k, e.target.value)} /><span className="hint">k tokens a day</span></>}
    </Row>
  )
  return (
    <Group title="Limits" note={msg ?? 'Caps on what routines and the people you share bots with use in a day. A routine that would go over pauses until midnight, and Needs you asks whether to let it go on today. Replies to your own messages are never stopped. Tokens read back from the AI’s cache count a tenth, as they’re billed; each routine shows what it uses on average.'}>
      {field('routine', 'Each routine', 'What one routine may use in a day.')}
      {field('bot', 'Each bot', 'What one bot may use in a day, replies included; only its routines pause.')}
      {field('day', 'All bots together', 'Everything, every bot, in a day.')}
      {field('person', 'Each person', 'What replies to one guest or family member may use in a day, in their own chat and a family group together. Over it they get a short note, and Needs you asks whether to allow more today.')}
      <Row icon={<Icon name="clock" size={15} />} title="Fresh start after a quiet spell"
        desc="When a Telegram, Slack or Discord chat has been quiet this long, the next message starts a new conversation with a short note on where you left off. Each reply re-sends less. 0: never.">
        {fresh !== null && <><input className="input mono" type="number" min={0} max={720} aria-label="Fresh start after" style={{ width: 72 }} defaultValue={fresh}
          onBlur={e => invoke<number>('fresh_after_hours', { hours: Math.max(0, Math.round(Number(e.target.value) || 0)) }).then(setFresh, er => setMsg(String(er)))} /><span className="hint">hours</span></>}
      </Row>
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
  { kind: 'anthropic', title: 'Anthropic API key', placeholder: 'sk-ant-…', hint: <>For Claude bots when the plan says no (set on each bot’s page). Create one in the Console organisation linked to your Max or Team plan, at <a href="https://console.anthropic.com/settings/keys" onClick={openLink}>console.anthropic.com</a>, to spend its monthly API credits.</> },
  { kind: 'openai', title: 'OpenAI-compatible API key', placeholder: 'sk-…', hint: 'For bots on the “API key” account: OpenAI, or the service at the bot’s endpoint.' },
]

function ApiKeySettings() {
  const [has, setHas] = useState<Record<string, boolean>>({})
  const [grok, setGrok] = useState<Account | null>(null)
  const [chatgpt, setChatgpt] = useState<Account | null>(null)
  const [xai, setXai] = useState<Account | null>(null)
  const loadGrok = () => invoke<Record<string, Account>>('accounts').then(a => { setGrok(a?.grok ?? null); setChatgpt(a?.chatgpt ?? null); setXai(a?.xai ?? null) }, () => {})
  useEffect(() => { loadGrok() }, [])
  const [open, setOpen] = useState<string | null>(null)
  const [key, setKey] = useState('')
  const [msg, setMsg] = useState<Record<string, string>>({})
  const [err, setErr] = useState<string | null>(null)
  const load = () => invoke<Record<string, boolean>>('api_keys').then(h => setHas(h ?? {}), e => setErr(String(e)))
  useEffect(() => { load() }, [])
  const save = (kind: string, k: string) => invoke('set_api_key', { kind, key: k }).then(() => { load(); loadGrok() })
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
      <Row icon={<Icon name="user" size={15} />} title="Grok plan on xAI’s API"
        desc={!xai ? 'Checking…' : xai.account ? `Signed in (${xai.account}). Bots on the xAI account use your plan${xai.key ? ' once the xAI key below is removed' : ''}.` : 'Sign in to run bots on the xAI account with your X Subscription, SuperGrok or X Premium+ plan. Nothing to install'}>
        {xai?.account ? <button className="btn btn-sm btn-ghost btn-danger" onClick={() => invoke('grok_plan_sign_out').then(loadGrok, e => setErr(String(e)))}>Sign out</button>
          : xai && <SignInGrok plan onDone={loadGrok} />}
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

type Mailbox = { user: string; host: string; smtp: string; smtp_port: number; outlook: boolean }

/** Your mailboxes for the bots (email_search, email_read, drafts, sending): IMAP with an app password, or Outlook with
 *  Microsoft's sign-in through your own app. Saving signs in once. */
function EmailSettings() {
  const st = useLoad(() => invoke<{ accounts: Mailbox[]; outlook_client_id: string; outlook_tenant: string }>('email_accounts'), [])
  const tasks = useLoad(() => invoke<boolean>('email_tasks', {}), [])
  const disclose = useLoad(() => invoke<string>('email_disclose', {}), [])
  const [line, setLine] = useState<string | null>(null)
  const setDisclose = (l: string) => invoke('email_disclose', { line: l }).then(() => { setLine(null); return disclose.reload() }, e => setErr(String(e)))
  const empty = { host: '', port: '993', user: '', password: '', smtp: '', smtpPort: '' }
  const [d, setD] = useState(empty)
  const [adding, setAdding] = useState<'password' | 'outlook' | null>(null)
  const [ms, setMs] = useState<{ clientId: string; tenant: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const list = st.data?.accounts ?? []
  const app = ms ?? { clientId: st.data?.outlook_client_id ?? '', tenant: st.data?.outlook_tenant ?? '' }
  const added = (r: { user: string; messages: number }) => { setMsg(`${r.user} signed in · ${r.messages} messages in the inbox`); setAdding(null); setErr(null); return st.reload() }
  const save = () => {
    setBusy(true); setErr(null)
    invoke<{ user: string; messages: number }>('email_account_save', { host: d.host.trim(), port: Number(d.port) || 993, user: d.user.trim(), password: d.password, smtpHost: d.smtp.trim(), smtpPort: Number(d.smtpPort) || undefined })
      .then(r => { setD(empty); return added(r) }, e => setErr(String(e)))
      .finally(() => setBusy(false))
  }
  const remove = (u: string) => ask(`Remove ${u}? Your bots can’t read or send from it any more.`, 'Remove', true)
    .then(yes => { if (yes) void invoke('email_account_remove', { user: u }).then(() => { setMsg(null); return st.reload() }, e => setErr(String(e))) })
  const mode = adding ?? (list.length ? null : 'password')
  return (
    <Group title="Email" note={err ? <span className="error">{err}</span> : `Lets your bots search and read your email in a chat with you, never in a group, write drafts, and send an email when you approve it (every one asks you, at most 20 a day). Nothing is marked read, moved or deleted. Emails count as outside content. Passwords and sign-ins are stored in ${SECRETS_AT}.`}>
      {list.map(m => (
        <Row key={m.user} icon={<Icon name="inbox" size={15} />} title={m.user}
          desc={m.outlook ? 'Outlook · signed in with Microsoft' : `Read through ${m.host}${m.smtp ? ` · sent through ${m.smtp}${m.smtp_port && m.smtp_port !== 465 ? `:${m.smtp_port}` : ''}` : ''}`}>
          <button className="btn btn-sm btn-ghost btn-danger" onClick={() => remove(m.user)}>Remove</button>
        </Row>
      ))}
      {list.length > 0 && (
        <Row title="Tasks by email" desc={`Mail you send to ${list[0].user.replace('@', '+mimir@')}, or file in a folder called Mimir, goes to your bot as a message; it answers in your latest chat with it. Only mail from your own address that passes DKIM is taken. What you forward is outside content: your note on top is what the bot does.${list.length > 1 ? ' Uses your first mailbox.' : ''}`}>
          <Switch label="Tasks by email" checked={!!tasks.data} onChange={on => invoke('email_tasks', { on }).then(() => tasks.reload(), e => setErr(String(e)))} />
        </Row>
      )}
      {list.length > 0 && (
        <Row title="Say it's an assistant" desc="An email a bot sends to someone you haven't written to yourself ends with this line. A reply to a bot's email that's personal, off-topic or asks you to commit to something comes to you in Needs you, not to the bot.">
          {!!disclose.data && <input className="input" style={{ width: 220 }} aria-label="The line added" value={line ?? disclose.data} onChange={e => setLine(e.target.value)} onBlur={() => line !== null && line.trim() && void setDisclose(line)} />}
          <Switch label="Say it's an assistant" checked={!!disclose.data} onChange={on => void setDisclose(on ? 'Sent by my assistant' : '')} />
        </Row>
      )}
      {!mode && (
        <Row icon={<Icon name="plus" size={15} />} title="Add another mailbox" desc={msg ?? 'Bots search all of them, and you say which one to send from.'}>
          <button className="btn btn-sm" onClick={() => { setAdding('password'); setMsg(null) }}>Add</button>
        </Row>
      )}
      {mode && (
        <div className="row-block">
          <Segmented<'password' | 'outlook'> label="How it signs in" size="sm" value={mode} onChange={setAdding} options={[{ id: 'password', label: 'App password' }, { id: 'outlook', label: 'Outlook / Microsoft 365' }]} />
        </div>
      )}
      {mode === 'password' && (
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
              {list.length > 0 && <button type="button" className="btn btn-ghost" onClick={() => setAdding(null)}>Cancel</button>}
            </div>
          </div>
          <span className="hint">Use an app password, not your normal one: Gmail and iCloud make one under your account's security settings (two-step sign-in has to be on). Gmail is imap.gmail.com, iCloud imap.mail.me.com, Fastmail imap.fastmail.com.</span>
        </form>
      )}
      {mode === 'outlook' && (
        <div className="row-block">
          <div className="editor" style={{ margin: 0 }}>
            <div className="er">
              <input className="input mono" placeholder="Application (client) ID" aria-label="Application (client) ID" autoComplete="off" value={app.clientId} onChange={e => setMs({ ...app, clientId: e.target.value })} />
              <input className="input mono" style={{ width: 200 }} placeholder="Tenant (optional)" aria-label="Directory (tenant) ID" title="Only for a work app that accepts just your organisation: its Directory (tenant) ID" autoComplete="off" value={app.tenant} onChange={e => setMs({ ...app, tenant: e.target.value })} />
              <SignInBrowser<{ user: string; messages: number }> label="Sign in with Microsoft" what="Outlook" disabled={!app.clientId.trim()}
                start={() => invoke('outlook_sign_in', { clientId: app.clientId.trim(), tenant: app.tenant.trim() })}
                wait="outlook_sign_in_wait" paste="outlook_sign_in_paste" onDone={added} />
              {list.length > 0 && <button type="button" className="btn btn-ghost" onClick={() => setAdding(null)}>Cancel</button>}
            </div>
          </div>
          <span className="hint">Microsoft turned off app passwords for mail, so Mimir signs in through an app you register once (free): in the <a href="https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade" onClick={openLink}>Azure portal</a>, App registrations → New registration, any account type, redirect <b>Public client/native</b> <code>http://localhost</code>. Paste its Application (client) ID here. <a href="https://github.com/currentbits/mimir/blob/master/docs/connect.md#outlook" onClick={openLink}>Step by step</a></span>
        </div>
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

/** Whether the tidy-up and imports may suggest memories about health, beliefs, money and the like. */
function SensitiveRow() {
  const [on, setOn] = useState<boolean | null>(null)
  useEffect(() => { invoke<boolean>('remember_sensitive').then(setOn, () => {}) }, [])
  return (
    <Row icon={<Icon name="memory" size={15} />} title="Remember sensitive topics"
      desc={on ? 'The tidy-up and imports may suggest memories about your health, beliefs, politics, sexuality and finances.' : 'The tidy-up and imports leave out your health, beliefs, politics, sexuality and finances (account numbers too), unless you say “remember that…” yourself.'}>
      {on !== null && <Switch label="Remember sensitive topics" checked={on} onChange={v => { setOn(v); invoke<boolean>('remember_sensitive', { on: v }).then(setOn, () => setOn(!v)) }} />}
    </Row>
  )
}

/** Whether an instruction a bot learns ("reply in Dutch") is saved right away or suggested for you to accept. */
function AcceptInstructionsRow() {
  const [on, setOn] = useState<boolean | null>(null)
  useEffect(() => { invoke<boolean>('accept_instructions').then(setOn, () => {}) }, [])
  return (
    <Row icon={<Icon name="memory" size={15} />} title="Always accept instructions"
      desc={on ? 'Instructions your bots learn are saved right away. One picked up from a web page or email stays unconfirmed until you check it.' : 'An instruction a bot learns waits in Memory for you to accept, unless you said it as a rule (“always…”, “from now on…”).'}>
      {on !== null && <Switch label="Always accept instructions" checked={on} onChange={v => { setOn(v); invoke<boolean>('accept_instructions', { on: v }).then(setOn, () => setOn(!v)) }} />}
    </Row>
  )
}

/** Memories as Markdown files in a folder you pick (vault.rs): Obsidian, grep, git. What you change there is asked about. */
function MemoryFolderSettings() {
  const st = useLoad(() => invoke<string>('memory_folder', {}), [])
  const [dir, setDir] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  if (st.data == null) return null
  const save = (d: string) => invoke<string>('memory_folder', { dir: d }).then(() => { setDir(null); setErr(null); st.reload() }, e => setErr(String(e)))
  const value = dir ?? st.data
  return (
    <Group title="Memory" note={err ? <span className="error">{err}</span> : <>One Markdown file per memory, a folder per chat, kept up to date. Open it in Obsidian, search it, keep it in git. What you edit, add or delete there waits in Needs you as a suggestion: nothing changes until you accept it. Files already in the folder are left alone.{remoteServer() || !inDesktop ? ' The folder is on the server.' : ''}</>}>
      <form className="row-block" onSubmit={e => { e.preventDefault(); void save(value) }}>
        <div className="editor" style={{ margin: 0 }}><div className="er">
          <input className="input mono" placeholder="Also keep as files: a folder's full path, e.g. /Users/you/Notes/Mimir" aria-label="Memory folder" value={value} onChange={e => setDir(e.target.value)} />
          <button className="btn" disabled={value.trim() === st.data}>Save</button>
          {st.data && <button type="button" className="btn btn-ghost" onClick={() => save('')}>Stop</button>}
        </div></div>
      </form>
      <SensitiveRow />
      <AcceptInstructionsRow />
    </Group>
  )
}

/** Web search for every bot (web_search): Brave Search's key, or your own SearXNG. Claude's own search works without. */
function WebSearchSettings() {
  const st = useLoad(() => invoke<{ brave: boolean; x: boolean; searxng_url: string }>('search_settings', {}), [])
  const [key, setKey] = useState('')
  const [xKey, setXKey] = useState('')
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
      <Row icon={<Icon name="search" size={15} />} title="X (Twitter)"
        desc={s?.x ? 'A token is saved; bots can search posts from the last 7 days' : 'Your own X API token, so bots can search recent posts'}>
        {s?.x ? <button className="btn btn-sm btn-ghost btn-danger" onClick={() => invoke('set_api_key', { kind: 'x', key: '' }).then(done, e => setErr(String(e)))}>Remove</button>
          : <form className="er" style={{ gap: 8 }} onSubmit={e => { e.preventDefault(); if (xKey.trim()) invoke('set_api_key', { kind: 'x', key: xKey.trim() }).then(() => { setXKey(''); return done() }, er => setErr(String(er))) }}>
              <input className="input" type="password" style={{ width: 200 }} placeholder="X bearer token" aria-label="X bearer token" value={xKey} onChange={e => setXKey(e.target.value)} autoComplete="off" />
              <button className="btn btn-sm" disabled={!xKey.trim()}>Save</button>
            </form>}
      </Row>
      {s && !s.x && <p className="hint" style={{ margin: '4px 0 8px' }}>
        To get one: 1. Sign in at <a href="https://console.x.com" onClick={openLink}>console.x.com</a> with your X account.
        {' '}2. Buy a few dollars of credits (X charges per post read; a daily search is a few cents).
        {' '}3. Create a Project, then an App inside it.
        {' '}4. In the App's <b>Keys and tokens</b>, generate the <b>Bearer Token</b>, copy it (it's shown once) and paste it above.
        {' '}You don't need the other keys. <a href="https://github.com/currentbits/mimir/blob/master/docs/connect.md#x-search" onClick={openLink}>Step by step</a>
      </p>}
    </Group>
  )
}

/** Scanned PDFs read on this computer: a document model in Ollama turns the pages into text before the bot sees them. */
function ScanSettings() {
  const st = useLoad(() => invoke<{ model: string; url: string; default_url: string }>('ocr_settings', {}), [])
  const [model, setModel] = useState<string | null>(null)
  const [url, setUrl] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const save = (a: { model?: string; url?: string }) => invoke('ocr_settings', a).then(() => { setModel(null); setUrl(null); setErr(null); return st.reload() }, e => setErr(String(e)))
  const s = st.data
  return (
    <Group title="Scanned PDFs" note={err ? <span className="error">{err}</span> : 'Without a model, a scan’s pages go to the bot’s AI as photos: more of your plan per page, and the scan leaves this computer. With one, they’re read here first; a page it can’t read still goes as a photo.'}>
      <Row icon={<Icon name="file" size={15} />} title="Read scans with a local model"
        desc={<>An <a href="https://ollama.com" onClick={openLink}>Ollama</a> model that reads pages, e.g. a build of <a href="https://github.com/studio-dots-ai/dots.ocr" onClick={openLink}>dots.ocr</a> or any vision model you’ve pulled. Empty: off.</>}>
        <input className="input" style={{ width: 200 }} placeholder="Model name" aria-label="Local document model" disabled={!s}
          value={model ?? s?.model ?? ''} onChange={e => setModel(e.target.value)} onBlur={() => model !== null && save({ model })} />
      </Row>
      {!!s?.model && <Row icon={<Icon name="server" size={15} />} title="Ollama's address" desc="Where Ollama runs; usually this computer.">
        <input className="input" style={{ width: 230 }} placeholder={s.default_url} aria-label="Ollama address"
          value={url ?? s.url} onChange={e => setUrl(e.target.value)} onBlur={() => url !== null && save({ url })} />
      </Row>}
    </Group>
  )
}

function BrowserSettings({ onChange }: { onChange: () => void }) {
  const servers = useLoad(() => invoke<{ servers: { name: string }[] }>('tool_servers'), [], 60000)
  const vault = useLoad(() => invoke<VaultEntry[]>('vault'), [], 60000)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const botOf = useContext(Bots)
  const [draft, setDraft] = useState({ name: '', value: '', domains: '', totp: '' })
  const exists = (vault.data ?? []).some(v => v.name === draft.name.trim().toUpperCase())
  const on = !!servers.data?.servers.some(s => s.name === 'browser')
  const toggle = (v: boolean) => {
    setBusy(true); setErr(null)
    invoke('browser_enable', { enabled: v }).then(() => { servers.reload(); onChange() }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const save = () => invoke('vault_save', {
    entry: { name: draft.name.trim().toUpperCase(), domains: draft.domains.split(/[,\s]+/).filter(Boolean) }, value: draft.value, totp: draft.totp,
  }).then(() => { setDraft({ name: '', value: '', domains: '', totp: '' }); setErr(null); vault.reload() }, e => setErr(String(e)))
  return (
    <Group title="Browser" note="Opening a new page asks you first. Running page scripts is off. Needs Node.js; the first time downloads Chromium (~100 MB).">
      <Row icon={<Icon name="browser" size={15} />} title="Web browser" desc={busy ? (on ? 'Turning off…' : 'Setting up — downloading Chromium the first time…') : 'A real headless browser bots can use: open pages, click, fill in forms.'}>
        <Switch label="Web browser" checked={on} disabled={busy} onChange={toggle} />
      </Row>
      {(on || !!vault.data?.length) && (
        <>
          <Row icon={<Icon name="shield" size={15} />} title="Saved logins" desc="Bots type these by name; the browser fills in the real value, which the AI never sees. Each works only on its sites, and you approve every use. With an authenticator key, Mimir types the current code too (NAME_CODE)." />
          {(vault.data ?? []).map(v => (
            <Row key={v.name} title={<code>{v.name}</code>} desc={v.domains.join(', ') + (v.bot ? ` · ${botOf(v.bot).name}'s, from signing in` : '') + (v.totp ? ' · authenticator codes' : '')}>
              <button className="btn btn-sm btn-ghost icon-btn" aria-label={`Change ${v.name}`} title="Change, or add an authenticator key" onClick={() => setDraft({ name: v.name, value: '', domains: v.domains.join(', '), totp: '' })}><Icon name="pencil" size={14} /></button>
              <button className="btn btn-sm btn-ghost btn-danger icon-btn" aria-label={`Remove ${v.name}`} title="Remove" onClick={() => invoke('vault_delete', { name: v.name }).then(vault.reload, e => setErr(String(e)))}><Icon name="trash" size={14} /></button>
            </Row>
          ))}
          <form className="row-block" onSubmit={e => { e.preventDefault(); save() }}>
            <div className="editor" style={{ margin: 0 }}><div className="er">
              <input className="input mono" aria-label="Login name" placeholder="GITHUB_PASSWORD" value={draft.name} onChange={e => setDraft({ ...draft, name: e.target.value })} />
              <input className="input" aria-label="Secret value" type="password" placeholder="Password or token" value={draft.value} onChange={e => setDraft({ ...draft, value: e.target.value })} autoComplete="new-password" />
              <input className="input" aria-label="Allowed sites" placeholder="github.com" value={draft.domains} onChange={e => setDraft({ ...draft, domains: e.target.value })} />
              <input className="input" aria-label="Authenticator setup key (optional)" type="password" placeholder="Authenticator key (optional)" value={draft.totp} onChange={e => setDraft({ ...draft, totp: e.target.value })} autoComplete="off" />
              <button className="btn" disabled={!draft.name.trim() || !(draft.value || (exists && draft.totp)) || !draft.domains.trim()}>Save</button>
            </div></div>
          </form>
          {on && <ImportLogins onDone={vault.reload} />}
        </>
      )}
      {err && <div className="row-block"><p className="error" style={{ margin: 0 }}>{err}</p></div>}
    </Group>
  )
}

type ImportRow = { i: number; site: string; user: string; totp: boolean; name: string; exists: boolean }

/** Saved logins from a password manager's CSV export: a preview (sites and user names only), then the ones you tick. */
function ImportLogins({ onDone }: { onDone: () => void }) {
  const [csv, setCsv] = useState<string | null>(null)
  const [rows, setRows] = useState<ImportRow[]>([])
  const [pick, setPick] = useState<Set<number>>(new Set())
  const [bot, setBot] = useState<number>(0)
  const [msg, setMsg] = useState<string | null>(null)
  const bots = useLoad(() => invoke<Bot[]>('bots'), [], 60000)
  const open = (f: File | undefined) => f && f.text().then(text => invoke<ImportRow[]>('logins_import', { csv: text }).then(r => {
    setCsv(text); setRows(r); setPick(new Set(r.filter(x => !x.exists).map(x => x.i))); setMsg(null)
  }, e => setMsg(String(e))))
  const save = () => csv && invoke<{ saved: number }>('logins_import', { csv, pick: [...pick], botId: bot || null }).then(r => {
    setCsv(null); setRows([]); onDone()
    setMsg(`Saved ${r.saved} login${r.saved === 1 ? '' : 's'}. Delete the export file now: it has every password in plain text.`)
  }, e => setMsg(String(e)))
  const toggle = (i: number) => setPick(p => { const n = new Set(p); if (n.has(i)) n.delete(i); else n.add(i); return n })
  return (
    <div className="row-block">
      {!csv ? <label className="btn btn-sm btn-ghost" style={{ cursor: 'pointer' }}>
        Import from a password manager…
        <input type="file" accept=".csv,text/csv" style={{ display: 'none' }} onChange={e => { void open(e.target.files?.[0]); e.target.value = '' }} />
      </label> : <>
        <p className="hint" style={{ margin: '0 0 8px' }}>Export as CSV from 1Password, Chrome, Edge, Safari / Passwords, Firefox, Bitwarden or LastPass. Only sites and user names are shown here; each login works only on its site, and you approve every use.</p>
        <div style={{ maxHeight: 280, overflow: 'auto' }}>
          {rows.map(r => (
            <label key={r.i} className="row" style={{ cursor: r.exists ? 'default' : 'pointer' }}>
              <input type="checkbox" checked={pick.has(r.i)} disabled={r.exists} onChange={() => toggle(r.i)} />
              <div className="row-main"><div className="row-title">{r.site}</div>
                <div className="row-desc">{r.user || 'no user name'}{r.totp ? ' · authenticator codes' : ''}{r.exists ? ' · already saved' : ` · saved as ${r.name}`}</div></div>
            </label>
          ))}
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 8 }}>
          <select className="select" style={{ width: 'auto' }} aria-label="For which bot" value={bot} onChange={e => setBot(Number(e.target.value))}>
            <option value={0}>For every bot</option>
            {(bots.data ?? []).map(b => <option key={b.id} value={b.id}>Only {b.name}</option>)}
          </select>
          <button type="button" className="btn btn-sm" disabled={!pick.size} onClick={() => void save()}>Import {pick.size}</button>
          <button type="button" className="btn btn-sm btn-ghost" onClick={() => { setCsv(null); setRows([]) }}>Cancel</button>
        </div>
      </>}
      {msg && <p className="hint" style={{ margin: '8px 0 0' }}>{msg}</p>}
    </div>
  )
}

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
  // `app`: from the directory, how to register an app there; its client id and secret go to the sign-in, not the server.
  const [form, setForm] = useState<{ text: string; name: string; guess: string; env: EnvRow[]; editing: boolean; app?: string; id?: string; secret?: string } | null>(null)
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
  const conf = () => {
    if (!form || !parsed || typeof parsed === 'string') return null
    const rows = Object.fromEntries(form.env.filter(e => e.key.trim()).map(e => [e.key.trim(), e.value]))
    return { name: form.name.trim(), command: parsed.command, args: parsed.args, url: parsed.url, ...parsed.url ? { headers: rows } : { env: rows } }
  }
  // Its tools are shown next, so the person confirms which really only read.
  const [review, setReview] = useState<string | null>(null)
  const ready = (name: string) => (n: number) => { setMsg(`${name} is ready with ${n} tool${n === 1 ? '' : 's'}.`); setErr(null); setForm(null); setReview(name); list.reload(); onChange() }
  const save = async (server = conf()) => {
    if (!server) return
    setBusy(true); setErr(null)
    try { ready(server.name)(await invoke<number>('tool_server_save', { server })) } catch (e) { setErr(String(e)) } finally { setBusy(false) }
  }
  // A remote server that answered 401 (or whose sign-in ended) offers MCP's sign-in.
  const server = conf()
  const wantsSignIn = remote && !!server && !!err && /sign in/.test(err)
  const signIn = (conf: object, name: string, label = `Sign in to ${name}`, client?: [string, string | null]) => <SignInBrowser<number> label={label} what={name} disabled={!!client && !client[0].trim()}
    start={() => invoke('tool_server_sign_in', { server: conf, client })} wait="tool_server_sign_in_wait" paste="tool_server_sign_in_paste" onDone={ready(name)} />
  // Moves its pinned package to the release named on the button (one at least a week old). Starts it once, like Save.
  const update = async (s: ToolServer) => {
    setBusy(true); setErr(null)
    try { ready(s.name)(await invoke<number>('tool_server_update', { name: s.name })) } catch (e) { setErr(String(e)) } finally { setBusy(false) }
  }
  // From the directory: one that signs in is added right away (asking it lands on "Sign in to …" in the form);
  // one that takes a token opens the form with its header to fill in.
  const [browse, setBrowse] = useState<string | null>(null)
  const add = (d: Listed) => {
    const base = slug(d.name)
    const name = servers.some(s => s.name === base) ? `${base}_2` : base
    setBrowse(null); setErr(null); setMsg(null)
    setForm({ text: d.url, name, guess: name, env: d.header ? [{ key: d.header, value: '' }] : [], editing: false, app: d.app, id: '', secret: '' })
    if (!d.header && !d.app) void save({ name, command: '', args: [], url: d.url, headers: {} })
  }
  const words = (browse ?? '').toLowerCase().split(/\s+/).filter(Boolean)
  const listed = DIRECTORY.filter(d => words.every(w => `${d.name} ${d.category} ${d.desc}`.toLowerCase().includes(w)))
  // Anything else: the official MCP registry, only servers on their publisher's own domain (the backend filters).
  type Found = Listed & { domain: string }
  const [found, setFound] = useState<{ q: string; list?: Found[]; err?: string } | null>(null)
  const q = (browse ?? '').trim()
  const searchRegistry = () => {
    setFound({ q })
    invoke<Found[]>('tool_server_registry', { q }).then(list => setFound({ q, list: list.filter(d => !DIRECTORY.some(x => x.url === d.url)) }), e => setFound({ q, err: String(e) }))
  }
  const addRow = (d: Listed, desc: ReactNode = d.desc) => (
    <Row key={d.url} icon={<Icon name="plug" size={15} />} title={d.name} desc={desc}>
      {servers.some(s => s.url === d.url) ? <span className="hint">Added</span> : <button className="btn btn-sm" disabled={busy} onClick={() => add(d)}>Add</button>}
    </Row>
  )
  const remove = (s: ToolServer) => ask(`Remove ${s.name}? Your bots lose its tools.`, 'Remove', true).then(ok => { if (ok) void invoke('tool_server_delete', { name: s.name }).then(() => { setMsg(null); list.reload(); onChange() }, e => setErr(String(e))) })

  return (
    <Group title="Tool servers" note={<>Give your bots more abilities with <a href="https://github.com/modelcontextprotocol/servers" onClick={openLink}>MCP servers</a> that run on {here} or online: your files, GitHub, a database. Tools you confirm only read are allowed; anything else asks you first (see Permissions). Tokens are stored in {SECRETS_AT}.</>}>
      {servers.map(s => (
        <Row key={s.name} icon={<Icon name="plug" size={15} />} title={s.name} desc={<>{s.tools} tool{s.tools === 1 ? '' : 's'} · <code>{s.url || [s.command, ...s.args].join(' ')}</code></>}>
          {s.signed_in && signIn({ name: s.name, url: s.url, headers: Object.fromEntries(s.headers.map(k => [k, ''])) }, s.name, 'Sign in again')}
          {!!s.unconfirmed && review !== s.name && <button className="btn btn-sm" onClick={() => setReview(s.name)}>Review tools</button>}
          {s.update && <button className="btn btn-sm" disabled={busy} title="A release at least a week old; Mimir doesn't take newer ones on its own" onClick={() => update(s)}>{busy ? 'Updating…' : `Update to ${s.update}`}</button>}
          <button className="btn btn-sm" onClick={() => open(s)}>Edit</button>
          <button className="btn btn-sm btn-ghost btn-danger icon-btn" aria-label={`Remove ${s.name}`} title="Remove" onClick={() => remove(s)}><Icon name="trash" size={14} /></button>
        </Row>
      ))}
      {review && <ReadOnlyReview key={review} server={review} onDone={() => { setReview(null); list.reload(); onChange() }} />}
      {!form && (
        <Row icon={<Icon name="plus" size={15} />} title={servers.length ? 'Add another tool server' : 'Add a tool server'} desc={msg ?? 'Pick a popular one, or paste the setup line or JSON config from the server’s instructions.'}>
          <button className="btn btn-sm" aria-expanded={browse !== null} onClick={() => setBrowse(browse === null ? '' : null)}>Browse</button>
          <button className="btn btn-sm" onClick={() => { setBrowse(null); open() }}>Paste</button>
        </Row>
      )}
      {!form && browse !== null && (
        <div className="row-block" style={{ maxHeight: 460, overflowY: 'auto' }}>
          <input className="input" autoFocus placeholder="Search Stripe, Notion, GitHub…" aria-label="Search tool servers" value={browse} onChange={e => setBrowse(e.target.value)} />
          {[...new Set(listed.map(d => d.category))].map(c => (
            <Fragment key={c}>
              <div className="field-label"><span>{c}</span></div>
              {listed.filter(d => d.category === c).map(d => addRow(d))}
            </Fragment>
          ))}
          <span className="hint">Hosted by each company on its own site. Signing in happens on their page; Mimir keeps the sign-in, never your password.</span>
          {!!q && found?.q !== q && <span><button type="button" className="btn btn-sm btn-ghost" onClick={searchRegistry}><Icon name="search" size={13} />Search the MCP registry for “{q}”</button></span>}
          {found?.q === q && !found.list && !found.err && <span className="hint">Searching the MCP registry…</span>}
          {found?.q === q && found.err && <span className="hint warn"><Icon name="alert" size={13} />{found.err}</span>}
          {found?.q === q && found.list && <>
            <div className="field-label"><span>From the MCP registry</span></div>
            {found.list.map(d => addRow(d, <>{d.desc} · <code>{d.domain}</code></>))}
            <span className="hint">{found.list.length ? 'Each runs on the site shown, which its publisher proved is theirs; Mimir hasn’t checked them otherwise. Anything that changes something still asks you first.' : 'Nothing there that runs on its publisher’s own site. Use Paste with the address from the server’s instructions.'}</span>
          </>}
        </div>
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
          {err && <p className={wantsSignIn ? 'hint' : 'error'} style={{ margin: 0 }}>{err}</p>}
          {form.app && server && <div className="field-label"><span>Your app’s sign-in</span>
            <span className="hint">{form.app}</span>
            <div className="er">
              <input className="input mono" style={{ flex: 1 }} aria-label="Client ID" placeholder="Client ID" value={form.id} onChange={e => setForm({ ...form, id: e.target.value })} />
              <input className="input" style={{ flex: 1 }} type="password" aria-label="Client secret" placeholder="Client secret" autoComplete="new-password" value={form.secret} onChange={e => setForm({ ...form, secret: e.target.value })} />
            </div>
            <span>{signIn(server, server.name || 'the server', undefined, [form.id ?? '', form.secret || null])}</span>
          </div>}
          {wantsSignIn && !form.app && <div className="er">{signIn(server, server.name || 'the server')}<span className="hint">You sign in in your browser; Mimir keeps the sign-in, never your password.</span></div>}
        </form>
      )}
      {err && !form && <div className="row-block"><p className="error" style={{ margin: 0 }}>{err}</p></div>}
    </Group>
  )
}

/** What you mark sensitive: once a run reads from one, everything that goes out asks. Suggested, never set, for banks and health. */
function SensitiveSettings() {
  const list = useLoad(() => invoke<string[]>('sensitive'), [], 60000)
  const servers = useLoad(() => invoke<{ servers: ToolServer[] }>('tool_servers'), [], 60000)
  const chats = useLoad(() => invoke<Chat[]>('chats'), [], 60000)
  const rules = useLoad(() => invoke<Rule[]>('rules'), [], 60000)
  const [err, setErr] = useState<string | null>(null)
  const on = new Set(list.data ?? [])
  const set = (name: string, v: boolean) => invoke('set_sensitive', { name, on: v }).then(() => { setErr(null); list.reload() }, e => setErr(String(e)))
  const toggle = (name: string, label: string) => <Switch label={`${label} is sensitive`} checked={on.has(name)} onChange={v => set(name, v)} />
  const srv = (servers.data?.servers ?? []).filter(s => s.name !== 'browser')
  const scopes = (chats.data ?? []).filter(c => c.memories > 0)
  const tools = (rules.data ?? []).map(r => r.tool).filter(t => t.includes('__') && !t.startsWith('browser__'))
  if (!srv.length && !scopes.length) return null
  return (
    <Group title="Sensitive sources" note={err ?? 'Your bank, your health records. Once a reply or routine has read from one, everything that goes out asks first, saying why: an email, a web page or search, a task for another bot, a message to another chat, and any tool server’s change.'}>
      {srv.map(s => (
        <Row key={s.name} icon={<Icon name="plug" size={15} />} title={s.name}
          desc={s.suggest_sensitive && !on.has(s.name) ? 'Looks like banking or health: mark it sensitive?' : `Tool server · ${s.tools} tool${s.tools === 1 ? '' : 's'}`}>{toggle(s.name, s.name)}</Row>
      ))}
      {scopes.map(c => (
        <Row key={c.chat_id} icon={<Icon name="user" size={15} />} title={`${c.title}’s memory`} desc={`${c.memories} memor${c.memories === 1 ? 'y' : 'ies'}`}>{toggle(`chat:${c.chat_id}`, `${c.title}’s memory`)}</Row>
      ))}
      {[...on].filter(n => n.includes('__')).map(t => (
        <Row key={t} icon={<Icon name="plug" size={15} />} title={toolLabel(t)} desc={`One tool of ${t.split('__')[0]}`}>{toggle(t, toolLabel(t))}</Row>
      ))}
      {tools.some(t => !on.has(t)) && (
        <Row icon={<Icon name="plus" size={15} />} title="Just one tool" desc="When only some of a server’s tools read private things.">
          <select className="select" style={{ width: 'auto' }} value="" aria-label="Mark one tool sensitive" onChange={e => e.target.value && set(e.target.value, true)}>
            <option value="">Pick a tool</option>
            {tools.filter(t => !on.has(t)).map(t => <option key={t} value={t}>{t.split('__')[0]}: {toolLabel(t)}</option>)}
          </select>
        </Row>
      )}
    </Group>
  )
}

/** A server's tools, and which it says only read. Those run without asking, but only once you confirm them here:
 *  a server could call a tool that changes things read-only. */
function ReadOnlyReview({ server, onDone }: { server: string; onDone: () => void }) {
  type Tool = { name: string; description: string; says_read_only: boolean; read_only: boolean }
  const tools = useLoad(() => invoke<Tool[]>('tool_server_tools', { name: server }), [server])
  const [on, setOn] = useState<string[] | null>(null)
  const list = tools.data ?? []
  const picked = on ?? list.filter(t => t.says_read_only).map(t => t.name)
  const [err, setErr] = useState<string | null>(null)
  const confirm = (names: string[]) => invoke('tool_server_read_only', { name: server, tools: names }).then(onDone, e => setErr(String(e)))
  if (!list.length) return null
  return (
    <div className="row-block">
      <p className="hint" style={{ margin: 0 }}>{server}’s tools. Those it says only read run without asking; turn off any that could change, send or delete something. The rest ask first.</p>
      {list.map(t => (
        <Row key={t.name} title={toolLabel(`${server}__${t.name}`)} desc={t.says_read_only ? `Read-only (the server says)${t.description ? ` · ${t.description}` : ''}` : t.description || undefined}>
          {t.says_read_only && <Switch label={`${t.name} is read-only`} checked={picked.includes(t.name)} onChange={c => setOn(c ? [...picked, t.name] : picked.filter(n => n !== t.name))} />}
        </Row>
      ))}
      <div className="actions">
        <button type="button" className="btn btn-ghost" onClick={() => confirm([])}>None are read-only</button>
        <span className="grow" />
        <button type="button" className="btn btn-primary" onClick={() => confirm(picked)}>Confirm</button>
      </div>
      {err && <p className="error" style={{ margin: 0 }}>{err}</p>}
    </div>
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

export const TOOL_LABEL: Record<string, string> = {
  memory_store: 'Remember things', memory_search: 'Look up memories', memory_forget: 'Forget a memory',
  routine_create: 'Create routines', routine_list: 'List routines', routine_delete: 'Delete a routine', web_fetch: 'Read web pages',
  chat_search: 'Search earlier conversations', meeting_search: 'Search your meetings', meeting_read: 'Read a meeting transcript', ask_bot: 'Hand tasks to other bots', skill_view: 'Read its skills', plan_propose: 'Propose a plan to approve once',
  ask_choice: 'Ask with buttons', progress_set: 'Keep a checklist', bot_create: 'Make new bots', calendar_add: 'Give calendar links',
  calendar_create: 'Add calendar events', calendar_list: 'Look at your calendar', card_write: 'Update pinned cards',
  computer_click: 'Click on the computer', computer_handoff: 'Ask you to take over the computer',
  computer_key: 'Press keys on the computer', computer_open: 'Open sites on the computer',
  computer_screenshot: "Look at the computer's screen", computer_scroll: 'Scroll on the computer',
  computer_signin: 'Use saved logins', computer_type: 'Type on the computer', email_draft: 'Draft emails', email_hand_over: 'Hand replies to you',
  email_read: 'Read your email', email_search: 'Search your email', email_send: 'Send emails',
  file_delete: 'Delete files', file_export: 'Copy files to your folders', mail_send: 'Send email from Mail on your Mac', messages_send: 'Send messages from your Mac', app_open: 'Open apps on your Mac', file_list: 'List files', file_read: 'Read files', file_write: 'Write files',
  goal_set: 'Set goals', goal_update: 'Update goals', routine_update: 'Change routines', send_file: 'Send files',
  skill_save: 'Save skills', web_search: 'Search the web', x_search: 'Search X',
}

/** "browser__browser_click" → "Click" (grouped under Browser) */
export const toolLabel = (t: string) => TOOL_LABEL[t] ?? (t.includes('__') ? t.split('__')[1].replace(/^browser_/, '').replaceAll('_', ' ').replace(/^./, c => c.toUpperCase()) : t)

type TargetRule = { kind: 'email' | 'folder'; pattern: string; action: 'ask' | 'deny'; label?: string }

/** Rules about what an action is for: emails to someone, or files copied into a folder. They hold over Always and "Don't ask me first". */
function AskFirstRules() {
  const rules = useLoad(() => invoke<TargetRule[]>('target_rules'), [], 60000)
  const [draft, setDraft] = useState<TargetRule>({ kind: 'email', pattern: '', action: 'ask' })
  const [err, setErr] = useState<string | null>(null)
  const all = rules.data ?? []
  const save = (next: TargetRule[]) => invoke<TargetRule[]>('target_rules', { rules: next.map(({ kind, pattern, action }) => ({ kind, pattern, action })) })
    .then(() => { setErr(null); setDraft(d => ({ ...d, pattern: '' })); rules.reload() }, e => setErr(String(e)))
  return (
    <Group title="Ask first rules" note={err ?? 'Rules about who or where, on top of Permissions: every bot asks (or never does it) when one matches, even with Always or “Don’t ask me first”. For emails: an address (dana@acme.com), a domain (acme.com), or !acme.com for anyone outside it.'}>
      {all.map((r, i) => (
        <Row key={r.kind + r.pattern + r.action} icon={<Icon name="shield" size={15} />} title={r.label ?? r.pattern} desc={r.action === 'deny' ? 'Never done' : 'Asks you first'}>
          <button type="button" className="btn btn-sm btn-ghost btn-danger" onClick={() => save(all.filter((_, j) => j !== i))}>Remove</button>
        </Row>
      ))}
      <form style={{ display: 'flex', gap: 8, alignItems: 'center' }} onSubmit={e => { e.preventDefault(); if (draft.pattern.trim()) void save([...all, draft]) }}>
        <select className="select" style={{ width: 'auto' }} aria-label="Rule about" value={draft.kind} onChange={e => setDraft({ ...draft, kind: e.target.value as TargetRule['kind'] })}>
          <option value="email">Emails to</option>
          <option value="folder">Files copied into</option>
        </select>
        <input className="input" aria-label="Who or where" placeholder={draft.kind === 'email' ? '!acme.com' : '~/Documents'} value={draft.pattern} onChange={e => setDraft({ ...draft, pattern: e.target.value })} autoComplete="off" spellCheck={false} />
        <Segmented size="sm" label="Action" value={draft.action} onChange={v => setDraft({ ...draft, action: v })} options={[{ id: 'ask', label: 'Ask' }, { id: 'deny', label: 'Never' }]} />
        <button type="submit" className="btn btn-sm" disabled={!draft.pattern.trim()}>Add</button>
      </form>
    </Group>
  )
}

/** Folders bots may copy files into (file_export, which asks every time). None: any folder of yours. */
function ExportFolders() {
  const folders = useLoad(() => invoke<string[]>('export_folders'), [], 60000)
  const [add, setAdd] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const all = folders.data ?? []
  const save = (next: string[]) => invoke<string[]>('set_export_folders', { folders: next }).then(() => { setErr(null); setAdd(''); folders.reload() }, e => setErr(String(e)))
  const pickHere = inDesktop && !remoteServer()
  return (
    <Group title="Export folders" note={err ?? `Where bots may copy files with file_export, on ${pickHere ? 'this computer' : 'the machine Mimir runs on'}; each copy still asks you. With none here, any folder of yours (never system folders, your home folder itself, or ~/.ssh-style ones).`}>
      {all.map(f => (
        <Row key={f} icon={<Icon name="file" size={15} />} title={f}>
          <button type="button" className="btn btn-sm btn-ghost btn-danger" onClick={() => save(all.filter(x => x !== f))}>Remove</button>
        </Row>
      ))}
      <form style={{ display: 'flex', gap: 8, alignItems: 'center' }} onSubmit={e => { e.preventDefault(); if (add.trim()) void save([...all, add.trim()]) }}>
        <input className="input" aria-label="Add an export folder" placeholder="~/.claude/skills" value={add} onChange={e => setAdd(e.target.value)} autoComplete="off" spellCheck={false} />
        {pickHere && <button type="button" className="btn btn-sm btn-ghost" onClick={() => localInvoke<string | null>('pick_folder').then(p => { if (p) void save([...all, p]) }, e => setErr(String(e)))}>Choose…</button>}
        <button type="submit" className="btn btn-sm" disabled={!add.trim()}>Add</button>
      </form>
    </Group>
  )
}

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
    <Row key={r.tool} title={toolLabel(r.tool)} desc={r.action === 'always' ? 'Never asks, not even after a bot reads a web page (Always on a card)' : r.action !== r.default ? `Changed from “${r.default}”` : r.default === 'deny' ? 'Denied by default: it can run any code in a page or upload your files' : undefined}>
      {/* Always is set from a card; here it shows, and goes back to Allow, Ask or Deny. */}
      <Segmented size="sm" label={toolLabel(r.tool)} value={r.action} onChange={a => set(r.tool, a)}
        options={[...(r.action === 'always' ? [{ id: 'always', label: 'Always' }] : []), { id: 'allow', label: 'Allow' }, { id: 'ask', label: 'Ask' }, { id: 'deny', label: 'Deny' }]} />
    </Row>
  )
  const sites = useLoad(() => invoke<{ tool: string; host: string; action: string }[]>('site_rules'), [], 60000)
  const removeSite = (tool: string, host: string) => invoke('set_site_rule', { tool, host, action: 'default' }).then(sites.reload, e => setErr(String(e)))
  const setAll = async (action: 'allow' | 'default') => {
    const msg = action === 'allow'
      ? 'Allow every tool without asking? Sending an email, making a bot and using a saved login still ask every time, and after a bot reads outside content, anything that changes your data still asks.'
      : 'Put every tool back to how Mimir comes (deleting and opening sites ask, the rest is allowed)?'
    if (!await ask(msg, action === 'allow' ? 'Allow all' : 'Back to defaults', action === 'allow')) return
    // Allow all leaves a tool at Always as it is (that's already more than allow).
    for (const r of all) if (r.action !== (action === 'default' ? r.default : 'allow') && !(action === 'allow' && r.action === 'always')) await invoke('set_rule', { tool: r.tool, action }).catch(e => setErr(String(e)))
    rules.reload()
  }
  return (
    <Group title="Permissions" action={<span className="perm-all">
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => setAll('default')} disabled={all.every(r => r.action === r.default)}>Back to defaults</button>
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => setAll('allow')} disabled={!all.length || all.every(r => r.action === 'allow' || r.action === 'always')}>Allow all</button>
    </span>} note={err ?? '“Ask” sends you Approve / Deny first. After a bot reads a web page, anything that changes your data asks, whatever is set here.'}>
      {builtin.map(row)}
      {(sites.data ?? []).map(r => (
        <Row key={r.tool + r.host} icon={<Icon name="globe" size={15} />} title={r.host}
          desc={`${r.tool === 'web_fetch' ? 'Read pages' : r.tool === 'computer_signin' ? 'Type its saved login' : r.tool.startsWith('browser__') ? 'Open in the browser' : toolLabel(r.tool)} · ${r.action === 'always' ? 'never asks, not even after a web page' : r.action === 'allow' ? 'always allowed' : r.action}`}>
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
                <div className="row-desc">{rs.length} tools{changed ? ` · ${changed} changed` : ''}{rs.some(r => r.default === 'deny' && r.action === 'deny') ? ' · unsafe ones denied by default' : ''}</div></div>
              <Icon name={openServer === server ? 'down' : 'chevron'} size={14} />
            </button>
            {openServer === server && rs.map(row)}
          </Fragment>
        )
      })}
    </Group>
  )
}
