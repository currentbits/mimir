import { useState } from 'react'
import { invoke } from '../api'
import { pigment, schedule, type Look } from '../format'
import { openLink } from '../markdown'
import { ask, Avatar, Group, Icon, Row, Switch } from '../ui'
import { ModelInput } from './accounts'
import { TOOL_LABEL } from './settings'
import { LOCAL, randomLook, useLoad, online, presenceOf, CHANNEL, LookPicker, SECRETS_AT } from '../shared'
import type { Bot, Delivery, Routine, Status, ToolServer, ChannelStatus } from '../types'

const parseLook = (s?: string): Look => { try { return s ? JSON.parse(s) : {} } catch { return {} } }

type Secrets = { telegram_token: string | null; slack_bot_token: string | null; slack_app_token: string | null; discord_token: string | null }

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

export const BLANK: Bot = { id: 0, name: '', persona: '', provider: 'claude', model: '', base_url: '', tools: '', has_telegram_token: false, has_slack: false, has_discord: false }

const NO_SECRETS: Secrets = { telegram_token: null, slack_bot_token: null, slack_app_token: null, discord_token: null }

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

export function BotPage({ bot, status, canDelete, onSaved, onDeleted, onChat }: {
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
  const topics = useLoad(() => bot ? invoke<boolean>('telegram_topics', { botId: bot.id }) : Promise.resolve(false), [bot?.id], 60000)
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
