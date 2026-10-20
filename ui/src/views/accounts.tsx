import { useEffect, useRef, useState } from 'react'
import { inDesktop, invoke, localInvoke, remoteServer } from '../api'
import { openLink } from '../markdown'
import { Icon } from '../ui'
import type { Account, Model } from '../types'

export const ACCOUNTS: Record<string, { name: string; mark: string; desc: string; install: string; signin: string }> = {
  claude: { name: 'Claude', mark: 'C', desc: 'Your Claude subscription, through Claude Code', install: 'Install Claude Code from claude.com/claude-code, then check again.', signin: 'Open Terminal, run claude, and sign in with your Claude account.' },
  chatgpt: { name: 'ChatGPT', mark: 'G', desc: 'Your ChatGPT Plus or Pro plan. Sign in; nothing to install', install: '', signin: 'Sign in with your ChatGPT account. You set how much of your plan Mimir may use in ChatGPT’s settings.' },
  codex: { name: 'Codex CLI', mark: 'G', desc: 'Your ChatGPT subscription, through the Codex CLI', install: 'Install the Codex CLI with npm install -g @openai/codex, then check again.', signin: 'Open Terminal and run codex login to sign in with ChatGPT.' },
  grok: { name: 'Grok', mark: 'X', desc: 'Your SuperGrok or X Premium+ subscription, through Grok Build', install: 'Install Grok Build with curl -fsSL https://x.ai/cli/install.sh | bash in Terminal, then check again.', signin: 'Sign in to Grok for Mimir. It keeps its own login, separate from your grok setup.' },
  openrouter: { name: 'OpenRouter', mark: 'R', desc: 'One key for Claude, GPT, Gemini, Llama and hundreds more', install: '', signin: '' },
  xai: { name: 'xAI API', mark: 'X', desc: 'Grok models on xAI’s API: your Grok plan, signed in, or an API key billed per token', install: '', signin: '' },
  local: { name: 'Local model', mark: 'L', desc: 'Ollama or LM Studio on this computer. Private and free.', install: "Install Ollama from ollama.com and run ollama pull qwen3:4b in Terminal (or start LM Studio's local server), then check again.", signin: 'Ollama is running but has no models yet. Run ollama pull qwen3:4b in Terminal, then check again.' },
  openai: { name: 'API key', mark: 'K', desc: 'OpenAI, or any OpenAI-compatible service', install: '', signin: '' },
}

export const ACCOUNT_ORDER = ['claude', 'chatgpt', 'codex', 'grok', 'openrouter', 'xai', 'local', 'openai']

/** Accounts that run on an API key, with what the key looks like. */
export const KEYED: Record<string, string> = { openrouter: 'OpenRouter key (sk-or-…)', xai: 'xAI key (xai-…)', openai: 'API key (sk-…)' }

const MODEL_HINT: Record<string, string> = {
  claude: 'Default — or sonnet, opus, haiku', chatgpt: 'Default — or one your plan offers', codex: 'Default — or one your plan offers', grok: 'Default — or grok-4.7-build-fast', openrouter: 'Auto — or pick one, e.g. anthropic/claude-sonnet-5.5',
  xai: 'grok-4.7', local: "Pick a model you've downloaded", openai: 'gpt-5-mini',
}

/** One-click OpenRouter sign-in (desktop app): you approve in the browser and the key comes back to the app. */
export function ConnectOpenRouter({ onDone, children = 'Connect OpenRouter' }: { onDone: () => void; children?: React.ReactNode }) {
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
export function SignInChatGPT({ onDone, place }: { onDone: () => void; place: (control: React.ReactNode, below: React.ReactNode) => React.ReactNode }) {
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

/** An OAuth sign-in in the browser (a tool server, Outlook): like ChatGPT's. `start` returns the address to open; the
 *  desktop app waits for the browser to come back (`wait`), a server takes the address it ended up on (`paste`). */
export function SignInBrowser<T>({ label, what, start, wait, paste, onDone, disabled }: {
  label: string; what: string; start: () => Promise<{ url: string; paste: boolean }>; wait: string; paste: string; onDone: (r: T) => void; disabled?: boolean
}) {
  const [flow, setFlow] = useState<{ url: string; paste: boolean } | null>(null)
  const [pasted, setPasted] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const run = useRef(0)
  const done = (r: T) => { run.current++; setFlow(null); setPasted(''); onDone(r) }
  const fail = (e: unknown) => { setBusy(false); setErr(String(e)) }
  const go = async () => {
    const me = ++run.current
    setBusy(true); setErr(null)
    try {
      const f = await start()
      setFlow(f); setBusy(false)
      if (inDesktop) localInvoke('open_url', { url: f.url }).catch(() => {})
      if (!f.paste) { const r = await invoke<T>(wait); if (me === run.current) done(r) }
    } catch (e) { if (me === run.current) { setFlow(null); fail(e) } }
  }
  const finish = () => { setErr(null); setBusy(true); invoke<T>(paste, { url: pasted.trim() }).then(r => { setBusy(false); done(r) }, fail) }
  const cancel = () => { run.current++; setFlow(null); setPasted(''); invoke('sign_in_cancel').catch(() => {}) }
  const error = err && <span className="error" style={{ margin: 0 }}>{err}</span>
  if (flow && !flow.paste) return (
    <span className="connect-wait" role="status"><span className="typing" aria-hidden="true"><i /><i /><i /></span>
      Sign in to {what} in your browser… <a href={flow.url} onClick={openLink}>Open it again</a>
      <button type="button" className="btn btn-sm btn-ghost" onClick={cancel}>Cancel</button></span>
  )
  if (flow) return (
    <div style={{ display: 'grid', gap: 6 }}>
      <span className="hint">1. <a href={flow.url} target="_blank" rel="noreferrer" onClick={openLink}>Sign in to {what}</a>{inDesktop ? ' (it opened in your browser)' : ''}.
        2. Your browser then lands on a page that doesn’t load, at an address starting with <code>http://</code>. Copy that whole address and paste it here.</span>
      <div className="er">
        <input className="input" placeholder="http://127.0.0.1:…?code=…" value={pasted} onChange={e => setPasted(e.target.value)} autoComplete="off" aria-label="The address your browser ended up on" autoFocus />
        <button type="button" className="btn" disabled={!pasted.trim() || busy} onClick={finish}>{busy ? 'Finishing…' : 'Finish'}</button>
        <button type="button" className="btn btn-ghost" onClick={cancel}>Cancel</button>
      </div>
      {error}
    </div>
  )
  return <>
    <button type="button" className="btn" disabled={busy || disabled} onClick={go}>{busy ? 'Starting…' : label}</button>
    {error}
  </>
}

/** Sign in to Grok for Mimir with a device code: open the page, confirm the code, and this notices when it's done.
 *  `plan`: the Grok plan on xAI's API (the xAI account, no Grok Build) instead of Grok Build's own login. */
export function SignInGrok({ onDone, plan = false }: { onDone: () => void; plan?: boolean }) {
  const [login, setLogin] = useState<{ url: string; code: string } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [start, stop] = plan ? ['grok_plan_sign_in', 'grok_plan_sign_in_cancel'] : ['grok_login', 'grok_login_cancel']
  useEffect(() => {
    if (!login) return
    const t = setInterval(() => invoke<Record<string, Account>>('accounts').then(a => {
      if (plan ? a?.xai?.account : a?.grok?.signed_in) { setLogin(null); (plan ? invoke('grok_plan_signed_in') : Promise.resolve()).finally(onDone) }
      else if (plan && a?.xai?.error) { setLogin(null); setErr(a.xai.error) }
    }, () => {}), 3000)
    return () => clearInterval(t)
  }, [login]) // eslint-disable-line react-hooks/exhaustive-deps
  const go = () => {
    setBusy(true); setErr(null)
    invoke<{ url: string; code: string }>(start).then(l => { setLogin(l); if (inDesktop) localInvoke('open_url', { url: l.url }).catch(() => {}) }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  if (login) return (
    <span className="connect-wait" role="status"><span className="typing" aria-hidden="true"><i /><i /><i /></span>
      Confirm <code>{login.code}</code> at <a href={login.url} target="_blank" rel="noreferrer" onClick={openLink}>{(() => { try { return new URL(login.url).hostname } catch { return login.url } })()}</a>…
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => { setLogin(null); invoke(stop).catch(() => {}) }}>Cancel</button></span>
  )
  return <>
    <button type="button" className="btn" disabled={busy} onClick={go}><Icon name="bolt" size={14} />{busy ? 'Starting…' : plan ? 'Sign in with your Grok plan' : 'Sign in to Grok'}</button>
    {err && <span className="error" style={{ margin: 0 }}>{err}</span>}
  </>
}

/** Model field with the account's models as suggestions. Warns about models that can't use tools. */
export function ModelInput({ provider, baseUrl, value, onChange, signedIn = true }: { provider: string; baseUrl?: string; value: string; onChange: (v: string) => void; signedIn?: boolean }) {
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
