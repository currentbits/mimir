import { useState } from 'react'
import { inDesktop, invoke, localInvoke, platform, remoteServer } from '../api'
import { pigment, schedule, type Look } from '../format'
import { openLink } from '../markdown'
import { ask, Avatar, Group, Icon, Row, Segmented, Switch } from '../ui'
import { ModelInput } from './accounts'
import { TOOL_LABEL, toolLabel, type Rule } from './settings'
import { LOCAL, randomLook, useLoad, online, presenceOf, CHANNEL, LookPicker, SECRETS_AT } from '../shared'
import type { Bot, Delivery, Routine, Status, ToolServer, ChannelStatus, ComputerStatus } from '../types'

const parseLook = (s?: string): Look => { try { return s ? JSON.parse(s) : {} } catch { return {} } }

// Messages is read on this Mac, so only the Mac app running its own bots can do it.
const imessageHere = inDesktop && platform === 'mac' && !remoteServer()

type Secrets = { telegram_token: string | null; slack_bot_token: string | null; slack_app_token: string | null; discord_token: string | null; signal_number: string | null; signal_api: string | null
  whatsapp_number_id: string | null; whatsapp_token: string | null; whatsapp_app_secret: string | null; imessage_chat: string | null }

const PROVIDERS = [
  { id: 'claude', name: 'Claude', hint: 'Your Claude login (Claude Code)' },
  { id: 'chatgpt', name: 'ChatGPT', hint: 'Your ChatGPT plan, signed in' },
  { id: 'codex', name: 'Codex CLI', hint: 'Your ChatGPT login, through the Codex CLI' },
  { id: 'grok', name: 'Grok', hint: 'Your SuperGrok login (Grok Build)' },
  { id: 'openrouter', name: 'OpenRouter', hint: 'Hundreds of models, one key' },
  { id: 'xai', name: 'xAI API', hint: 'Grok: your plan, or a key' },
  { id: 'local', name: 'Local model', hint: 'Ollama or LM Studio, free' },
  { id: 'openai', name: 'API key', hint: 'OpenAI or any compatible API' },
]

const PROVIDER_NOTE: Record<string, React.ReactNode> = {
  claude: <>Runs <code>claude -p</code> with your own login. Anthropic may count use by other apps as extra usage.</>,
  chatgpt: 'Runs on your ChatGPT Plus or Pro plan, signed in under Settings → AI keys. You set how much Mimir may use each week in ChatGPT’s settings.',
  openrouter: 'Uses your OpenRouter key from Settings. OpenRouter bills per message; the run log shows the cost.',
  local: "Runs on this computer (Ollama by default; set the endpoint for LM Studio, http://localhost:1234/v1). Chats and memories don't leave it.",
  grok: <>Runs <code>grok -p</code> on your SuperGrok or X Premium+ subscription. Sign in under Settings → AI keys.</>,
  xai: 'Uses your xAI key from Settings (xAI bills per token), or, without one, your Grok plan signed in under Settings → AI keys.',
  openai: 'Uses the API key from Settings.',
}

export const BLANK: Bot = { id: 0, name: '', persona: '', provider: 'claude', model: '', base_url: '', tools: '', has_telegram_token: false, has_slack: false, has_discord: false, has_signal: false, has_whatsapp: false, has_imessage: false }

const NO_SECRETS: Secrets = { telegram_token: null, slack_bot_token: null, slack_app_token: null, discord_token: null, signal_number: null, signal_api: null, whatsapp_number_id: null, whatsapp_token: null, whatsapp_app_secret: null, imessage_chat: null }

/** A routine a template starts with. Edit the prompt or switch it off before creating the bot. */
type StarterRoutine = { cron: string; prompt: string; quiet?: boolean; delivery?: Delivery; on: boolean }

/** Starting points, by what the bot is responsible for. Each fills in the role and offers routines. */
export const TEMPLATES: { id: string; icon: string; name: string; job: string; persona: string; routines: Omit<StarterRoutine, 'on'>[] }[] = [
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
export async function addStarterRoutines(botId: number, routines: Omit<StarterRoutine, 'on'>[]) {
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

type Flagged = { title: string; text: string; body: string; flags: string[] }
type BotFilePreview = {
  name: string; look: string; role: string; provider: string; model: string; tools: string; skills: Flagged[]; routines: Flagged[]
  rules: { tool: string; action: string; yours: string; looser: boolean }[]; servers: { name: string; what: string; here: boolean }[]; warnings: string[]
}

const Flags = ({ flags }: { flags: string[] }) => flags.length ? <span className="hint warn"><Icon name="alert" size={13} />It {flags.join(', ')}.</span> : null

/** What a bot file would add, before anything is: you read it, and Add bot adds it (skills as proposals, routines paused). */
export function BotImport({ text, duplicateOf, onClose, onAdded }: { text: string; duplicateOf?: number; onClose: () => void; onAdded: (id: number) => void }) {
  const p = useLoad(() => invoke<BotFilePreview>('import_bot_preview', { text, duplicateOf }), [text])
  const [rules, setRules] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const add = () => invoke<{ id: number }>('import_bot', { text, rules, duplicateOf }).then(r => onAdded(r.id), e => setErr(String(e)))
  const f = p.data
  return (
    <div className="ask-backdrop" onMouseDown={e => e.target === e.currentTarget && onClose()}>
      <div className="ask-box" role="dialog" aria-modal="true" aria-label="Add a bot from a file" onKeyDown={e => e.key === 'Escape' && onClose()}
        style={{ width: 'min(560px, calc(100vw - 32px))', maxHeight: 'calc(100vh - 64px)', overflow: 'auto' }}>
        {p.error || !f ? <p>{p.error ? String(p.error) : 'Reading the file…'}</p> : <>
          <div className="page-head">
            <Avatar id={99} name={f.name} size={44} look={parseLook(f.look)} />
            <div><b>{f.name}</b><p className="hint">{[f.provider, f.model].filter(Boolean).join(' · ')}{f.tools ? ` · ${f.tools.split(',').length} tools` : ' · all tools'}</p></div>
          </div>
          <p className="hint">Nothing is added until you press <b>Add bot</b>. Its skills and routines are someone else's words: read them first.</p>
          {f.warnings.map(w => <span key={w} className="hint warn"><Icon name="alert" size={13} />{w}</span>)}
          {f.role && <div><span className="field-label">Role</span><p className="selectable" style={{ whiteSpace: 'pre-wrap', maxHeight: 120, overflow: 'auto' }}>{f.role}</p></div>}
          {f.skills.length > 0 && <div><span className="field-label">Skills, as proposals to accept in Memory</span>
            <ul>{f.skills.map((k, i) => <li key={i}><b>{k.title}</b>: {k.text}<Flags flags={k.flags} />
              {k.body && <details className="inbox-raw"><summary>Show</summary><p className="selectable" style={{ whiteSpace: 'pre-wrap', maxHeight: 240, overflow: 'auto' }}>{k.body}</p></details>}</li>)}</ul></div>}
          {f.routines.length > 0 && <div><span className="field-label">Routines, paused until you resume them</span>
            <ul>{f.routines.map((r, i) => <li key={i}><b>{schedule(r.title)}</b>: {r.text}{r.body && <span className="hint">{r.body}</span>}<Flags flags={r.flags} /></li>)}</ul></div>}
          {f.servers.length > 0 && <div><span className="field-label">Tool servers it uses</span>
            <ul>{f.servers.map(s => <li key={s.name}><b>{s.name}</b>: <code>{s.what}</code>{s.here ? '' : ' (not set up here: nothing from the file is run or added; set it up under Settings if you trust it)'}</li>)}</ul></div>}
          {f.rules.length > 0 && <div><span className="field-label">Its tool rules, different from yours</span>
            <ul>{f.rules.map(r => <li key={r.tool}><code>{r.tool}</code>: {r.action} (yours: {r.yours}){r.looser && <span className="hint warn"><Icon name="alert" size={13} />Looser than yours, so not used. Change it under Settings → Permissions if you want it.</span>}</li>)}</ul>
            {f.rules.some(r => !r.looser) && <label className="hint"><input type="checkbox" checked={rules} onChange={e => setRules(e.target.checked)} /> Use its stricter rules too (they apply to all your bots)</label>}</div>}
          {err && <span className="hint warn"><Icon name="alert" size={13} />{err}</span>}
        </>}
        <div className="ask-actions">
          <button type="button" className="btn btn-ghost" onClick={onClose}>Cancel</button>
          <button type="button" className="btn btn-primary" disabled={!f} onClick={add}>Add bot</button>
        </div>
      </div>
    </div>
  )
}

export function BotPage({ bot, status, canDelete, onSaved, onDeleted, onChat, onImport }: {
  bot?: Bot; status?: Status; canDelete: boolean; onSaved: (id: number) => void; onDeleted: () => void; onChat?: () => void
  onImport?: (text: string) => void
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
  const topics = useLoad(() => bot ? invoke<boolean>('telegram_topics', { botId: bot.id }) : Promise.resolve(false), [bot?.id], 60000)
  const handsOff = useLoad(() => bot ? invoke<boolean>('hands_off', { botId: bot.id }) : Promise.resolve(false), [bot?.id])
  const setHandsOff = async (on: boolean) => {
    if (on && !await ask(`Let ${bot!.name} act without asking you first? It deletes, sends email, opens sites, uses saved logins and makes bots with no card, even after it reads a web page or email that could be telling it to. Deny rules in Settings → Permissions still hold.`, 'Don’t ask me', true)) return
    invoke('set_hands_off', { botId: bot!.id, on }).then(handsOff.reload, e => setErr(String(e)))
  }
  const computer = useLoad(() => invoke<ComputerStatus>('computer'), [], 60000)
  const serverTools: [string, string][] = (toolServers.data?.servers ?? []).filter(s => !s.preset).map(s => [`${s.name}__*`, `Use ${s.name}`])
  const ALL = [...Object.keys(TOOL_LABEL), 'browser__*', ...serverTools.map(([t]) => t)]
  // What each tool does for this bot once it's on: Settings → Permissions (the same for every bot), defaults included.
  const rules = useLoad(() => invoke<Rule[]>('rules'), [], 60000)
  const RULE_WORDS: Record<string, string> = { always: 'never asks', allow: 'allowed', ask: 'asks you first', deny: 'denied' }
  const effective = (tool: string) => {
    const of = (r: Rule) => `${RULE_WORDS[r.action] ?? r.action}${r.action === r.default ? ' (default)' : ''}`
    if (!tool.endsWith('__*')) { const r = rules.data?.find(r => r.tool === tool); return r && of(r) }
    const rs = (rules.data ?? []).filter(r => r.tool.startsWith(tool.slice(0, -1)))
    const odd = rs.filter(r => r.action === 'deny' || r.action === 'ask')
    return rs.length ? odd.map(r => `${toolLabel(r.tool)}: ${of(r)}`).join(' · ') || 'all allowed' : undefined
  }
  const enabled = b.tools.trim() ? b.tools.split(',').map(t => t.trim()) : ALL
  const toggle = (tool: string) => {
    const next = enabled.includes(tool) ? enabled.filter(t => t !== tool) : [...enabled, tool]
    edit({ tools: ALL.every(t => next.includes(t)) ? '' : next.join(',') })
  }
  // Which channels are connected comes from the saved bot (it changes on save), not this form's copy.
  const has = (k: 'has_telegram_token' | 'has_slack' | 'has_discord' | 'has_signal' | 'has_whatsapp' | 'has_imessage') => !!bot?.[k]
  const form = (x: Bot) => JSON.stringify({ ...x, has_telegram_token: 0, has_slack: 0, has_discord: 0, has_signal: 0, has_whatsapp: 0, has_imessage: 0 })
  const dirty = form(b) !== form(bot ?? { ...BLANK, look: b.look }) || Object.values(sec).some(v => v !== null)
  // Its working folder: where it is now (and whether it's there), the system's picker on this computer, and whether
  // to copy its files in when you set one.
  const folder = useLoad(() => bot ? invoke<{ path: string; chosen: boolean; machine: string; problem: string | null; can_open: boolean }>('bot_folder', { botId: bot.id }) : Promise.resolve(null), [bot?.id, bot?.workdir], 60000)
  const [copyFiles, setCopyFiles] = useState(true)
  const newFolder = !!b.workdir?.trim() && b.workdir !== (bot?.workdir ?? '')
  const pickHere = inDesktop && !remoteServer()
  const pick = () => localInvoke<string | null>('pick_folder').then(p => { if (p) edit({ workdir: p }) }, e => setErr(String(e)))
  const save = (secrets: Secrets) =>
    invoke<number>('save_bot', { bot: b, secrets, copyFiles: newFolder && copyFiles })
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

      {isNew && <Group title="Start from" action={onImport && <label className="btn btn-ghost btn-sm"><Icon name="file" size={13} />From a file…
        <input type="file" accept=".json,application/json" hidden onChange={e => { void e.target.files?.[0]?.text().then(onImport); e.target.value = '' }} /></label>}><div className="row-block"><Templates picked={template} pick={applyTemplate} /></div></Group>}

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

      {bot && <Fallback bot={bot} onErr={e => { setErr(e); setMsg(null) }} />}

      <Group title="Messaging apps" note={`Each bot is its own Telegram, Slack, Discord, Signal or WhatsApp bot{imessageHere ? ', or answers in one iMessage conversation' : ''}. Tokens are stored in ${SECRETS_AT}.`}>
        <ChannelRow icon="telegram" title="Telegram" st={chan('telegram')} has={has('has_telegram_token')} onRemove={() => save({ ...NO_SECRETS, telegram_token: '' })}
          hint="Message @BotFather, send /newbot, and paste the token it gives you.">
          <input className="input" type="password" aria-label="Telegram bot token" placeholder="123456:ABC…" value={typed('telegram_token')} onChange={e => type('telegram_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
        {bot && has('has_telegram_token') && (
          <Row title="Each topic is its own conversation" desc="If you turn on topics in your private chat with this bot on Telegram, each topic gets its own conversation and /new. Memories and routines are shared. Telegram is still changing this feature.">
            <Switch label="Each topic is its own conversation" checked={!!topics.data} onChange={on => invoke('set_telegram_topics', { botId: bot.id, on }).then(topics.reload, e => setErr(String(e)))} />
          </Row>
        )}
        <ChannelRow icon="slack" title="Slack" st={chan('slack')} has={has('has_slack')} onRemove={() => save({ ...NO_SECRETS, slack_bot_token: '', slack_app_token: '' })}
          hint="Create a Slack app with Socket Mode and Interactivity on; scopes chat:write, im:history, app_mentions:read, files:read, files:write, channels:history, groups:history; events message.im and app_mention (add message.channels and message.groups so it follows a thread without another @-mention).">
          <input className="input" type="password" aria-label="Slack bot token" placeholder="Bot token (xoxb-…)" value={typed('slack_bot_token')} onChange={e => type('slack_bot_token', e.target.value)} autoComplete="off" />
          <input className="input" type="password" aria-label="Slack app-level token" placeholder="App-level token (xapp-…)" value={typed('slack_app_token')} onChange={e => type('slack_app_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
        <ChannelRow icon="discord" title="Discord" st={chan('discord')} has={has('has_discord')} onRemove={() => save({ ...NO_SECRETS, discord_token: '' })}
          hint="In the Discord Developer Portal, create a bot, turn on Message Content Intent, and invite it with Send Messages and Attach Files.">
          <input className="input" type="password" aria-label="Discord bot token" placeholder="Bot token" value={typed('discord_token')} onChange={e => type('discord_token', e.target.value)} autoComplete="off" />
        </ChannelRow>
        <ChannelRow icon="signal" title="Signal" st={chan('signal')} has={has('has_signal')} onRemove={() => save({ ...NO_SECRETS, signal_number: '', signal_api: '' })}
          hint="Run signal-cli-rest-api with MODE=json-rpc, link the bot's own Signal number to it (open /v1/qrcodelink?device_name=mimir on it and scan the code in Signal → Linked devices), then enter that number. Direct chats only.">
          <input className="input" aria-label="Signal number" placeholder="The bot's number (+31…)" value={typed('signal_number')} onChange={e => type('signal_number', e.target.value)} autoComplete="off" />
          <input className="input" aria-label="signal-cli-rest-api address" placeholder="signal-cli-rest-api address (default http://127.0.0.1:8081)" value={typed('signal_api')} onChange={e => type('signal_api', e.target.value)} autoComplete="off" />
        </ChannelRow>
        <ChannelRow icon="whatsapp" title="WhatsApp" st={chan('whatsapp')} has={has('has_whatsapp')} onRemove={() => save({ ...NO_SECRETS, whatsapp_number_id: '', whatsapp_token: '', whatsapp_app_secret: '' })}
          hint="Needs Mimir as a server at an https address. In a Meta app with WhatsApp, give it a number of its own and a system user's access token; set the webhook to your address + /whatsapp, with the phone number id as the verify token, and subscribe to messages. It can only write within 24 hours of your last message there.">
          <input className="input" aria-label="WhatsApp phone number id" placeholder="Phone number id (digits, from API Setup)" value={typed('whatsapp_number_id')} onChange={e => type('whatsapp_number_id', e.target.value)} autoComplete="off" />
          <input className="input" type="password" aria-label="WhatsApp access token" placeholder="Access token" value={typed('whatsapp_token')} onChange={e => type('whatsapp_token', e.target.value)} autoComplete="off" />
          <input className="input" type="password" aria-label="Meta app secret" placeholder="App secret (signs the webhook's calls)" value={typed('whatsapp_app_secret')} onChange={e => type('whatsapp_app_secret', e.target.value)} autoComplete="off" />
        </ChannelRow>
        {imessageHere && (
          <ChannelRow icon="imessage" title="iMessage" st={chan('imessage')} has={has('has_imessage')} onRemove={() => save({ ...NO_SECRETS, imessage_chat: '' })}
            hint="Answers in one Messages conversation on this Mac and never reads any other: a note to yourself (your own number or email), or a chat with a second Apple ID. Give Mimir Full Disk Access (System Settings → Privacy & Security), and allow it to control Messages when asked. Only while this Mac is awake. Choices come as numbered lists.">
            <input className="input" aria-label="iMessage conversation" placeholder="Their number (+31…) or email, or your own for a note to yourself" value={typed('imessage_chat')} onChange={e => type('imessage_chat', e.target.value)} autoComplete="off" />
          </ChannelRow>
        )}
      </Group>

      <Group title="What it can do" note={handsOff.data ? 'It uses these without asking you first (deleting or changing your own files in its working folder still asks).' : 'Under each: what Settings → Permissions decides for it (the same for every bot). After a bot reads a web page, anything that changes your data asks.'}>
        {bot && (
          <Row title="Don’t ask me first" desc="Its tools run without approval cards, also after it reads outside content, and instructions and skills it learns are kept without waiting for you to accept them. Signing in, taking over the computer, and deleting or changing your files in its working folder still need you; a guest’s chat still asks the guest.">
            <Switch label="Don’t ask me first" checked={!!handsOff.data} onChange={on => void setHandsOff(on)} />
          </Row>
        )}
        <div className="tool-grid">
          {[...Object.entries(TOOL_LABEL), ['browser__*', 'Use the web browser'], ...serverTools].map(([tool, label]) => (
            <Row key={tool} title={label} desc={enabled.includes(tool) ? effective(tool) : 'off for this bot'}><Switch label={label} checked={enabled.includes(tool)} onChange={() => toggle(tool)} /></Row>
          ))}
        </div>
      </Group>

      <Group title="Working folder" note={<>Where its files go (file_list, file_read, file_write), on {pickHere ? 'this computer' : folder.data?.machine ?? 'the machine Mimir runs on'}. Pick a folder of yours to see and edit them in {platform === 'mac' ? 'Finder' : 'your file manager'}; paths stay inside it (no ../, no links out). Empty: Mimir's own workspace for each chat.</>}>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <input className="input" aria-label="Working folder" placeholder={pickHere ? 'Mimir’s own workspace (default)' : 'A folder on the server, like /data/bots/scout'} value={b.workdir ?? ''} onChange={e => edit({ workdir: e.target.value })} autoComplete="off" spellCheck={false} />
          {pickHere && <button type="button" className="btn btn-sm btn-ghost" onClick={pick}>Choose…</button>}
          {!!b.workdir && <button type="button" className="btn btn-sm btn-ghost" onClick={() => edit({ workdir: '' })}>Use Mimir’s</button>}
        </div>
        {newFolder && <Row title="Copy its files there" desc="What it has in Mimir’s workspace (its own, and the app chat’s) is copied in when you save; nothing there is replaced."><Switch label="Copy its files there" checked={copyFiles} onChange={() => setCopyFiles(c => !c)} /></Row>}
        {folder.data && !newFolder && <Row title={folder.data.chosen ? 'Its folder' : 'Its own workspace (for tasks other bots hand it)'} desc={folder.data.problem ?? `${folder.data.path} on ${folder.data.machine}`}>
          {folder.data.can_open && !folder.data.problem && <button type="button" className="btn btn-sm btn-ghost" onClick={() => invoke('open_bot_folder', { botId: bot!.id }).catch(e => setErr(String(e)))}>{platform === 'mac' ? 'Reveal in Finder' : 'Show in Explorer'}</button>}
        </Row>}
        <Row title="Writing files" desc="Whether file_write asks you first, for this bot. Deleting a file, or changing one already in a folder you chose, always asks.">
          <Segmented size="sm" label="Writing files" value={b.file_write || 'default'} onChange={v => edit({ file_write: v === 'default' ? '' : v })}
            options={[{ id: 'default', label: 'As Permissions' }, { id: 'ask', label: 'Ask' }, { id: 'allow', label: 'Allow' }, { id: 'off', label: 'Off' }]} />
        </Row>
      </Group>

      {bot && computer.data?.enabled && (
        <Group title="Computer" note="Sites you sign in to on the computer for this bot stay signed in for this bot only.">
          <Row title="Its sign-ins on the computer" desc="Cookies and logins in its own browser on the computer">
            <button type="button" className="btn btn-sm btn-ghost btn-danger" onClick={() => ask(`Sign ${bot.name} out of every site on the computer?`, 'Sign out', true).then(ok => {
              if (ok) invoke('computer_forget', { botId: bot.id }).then(() => { setErr(null); setMsg('Signed out on the computer') }, e => { setErr(String(e)); setMsg(null) })
            })}>Sign out everywhere</button>
          </Row>
        </Group>
      )}

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

/** Logins: a fallback on the bot's own one is at the same limit. */
const LOGINS = ['claude', 'chatgpt', 'codex', 'grok']
const PAID = ['openrouter', 'xai', 'openai']

/** Where its routines run while its own AI account is at a usage limit, until that resets. Off by default. */
function Fallback({ bot, onErr }: { bot: Bot; onErr: (e: string) => void }) {
  type Fb = { provider: string; model: string }
  const saved = useLoad(() => invoke<Fb | null>('fallback', { botId: bot.id }), [bot.id])
  const [draft, setDraft] = useState<Fb | null>(null)
  const now = saved.data ?? { provider: '', model: '' }
  const fb = draft ?? now
  const changed = draft !== null && (draft.provider !== now.provider || draft.model.trim() !== now.model)
  const use = () => invoke('fallback', { botId: bot.id, provider: fb.provider, model: fb.model }).then(() => { setDraft(null); saved.reload() }, e => onErr(String(e)))
  return (
    <Group title="When it hits a usage limit" note={PAID.includes(fb.provider)
      ? 'This account bills per token: routines that run on it while your plan is at its limit cost money. Daily caps in Settings → Limits still apply.'
      : 'Routines run on the fallback until your plan’s limit resets, then on the bot’s own account again. The run log says which ran there. Daily caps still apply.'}>
      {bot.provider === 'claude' && <ApiCredits bot={bot} onErr={onErr} />}
      <div className="row-block">
        <div className="editor" style={{ margin: 0 }}>
          <div className="er" style={{ alignItems: 'flex-start' }}>
            <label className="field-label" style={{ flex: 1 }}><span>Fallback account</span>
              <select className="select" value={fb.provider} onChange={e => setDraft({ provider: e.target.value, model: '' })}>
                <option value="">None: routines wait until the limit resets</option>
                {PROVIDERS.filter(p => p.id !== bot.provider || !LOGINS.includes(p.id)).map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select></label>
            {fb.provider && <label className="field-label" style={{ flex: 1 }}><span>Model</span>
              <ModelInput provider={fb.provider} value={fb.model} onChange={model => setDraft({ ...fb, model })} /></label>}
          </div>
          {changed && <button type="button" className="btn btn-sm" onClick={use}>{fb.provider ? 'Use this fallback' : 'Turn off the fallback'}</button>}
        </div>
      </div>
    </Group>
  )
}

/** A Claude bot on the Anthropic API key from Settings → AI keys: a Max or Team plan's monthly API credits, which
 *  stop when they run out rather than bill. The same run goes on there, with its tools and conversation. */
function ApiCredits({ bot, onErr }: { bot: Bot; onErr: (e: string) => void }) {
  const mode = useLoad(() => invoke<string>('api_credits', { botId: bot.id }), [bot.id])
  const set = (m: string) => invoke('api_credits', { botId: bot.id, mode: m }).then(() => mode.reload(), e => onErr(String(e)))
  return (
    <Row icon={<Icon name="key" size={15} />} title="Anthropic API key"
      desc="Your Max or Team plan’s monthly API credits, with the key from your linked Claude Console organisation (Settings → AI keys). When they run out, requests stop: nothing is billed.">
      <select className="select" aria-label="Anthropic API key" value={mode.data ?? ''} disabled={mode.data === null} onChange={e => set(e.target.value)}>
        <option value="">Don’t use it</option>
        <option value="limit">When the plan says no</option>
        <option value="routines">For routines, and when the plan says no</option>
      </select>
    </Row>
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
