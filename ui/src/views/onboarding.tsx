import { useEffect, useRef, useState } from 'react'
import { inDesktop, invoke, remoteServer } from '../api'
import { schedule, type Look } from '../format'
import { openLink } from '../markdown'
import { Avatar, Icon } from '../ui'
import { ACCOUNT_ORDER, KEYED, ACCOUNTS, SignInGrok, SignInChatGPT, ConnectOpenRouter, ModelInput } from './accounts'
import { TEMPLATES, BLANK, addStarterRoutines } from './bot'
import { LOCAL, randomLook, LookPicker } from '../shared'
import type { Account, LibrarySkill, Bot } from '../types'

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
export function Onboarding({ onDone }: { onDone: () => void }) {
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
                  : KEYED[k] ? (a?.key === false && a.account ? ['Signed in', 'live'] : a?.signed_in ? ['Key saved', 'live'] : ['Bring your own', ''])
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
            {provider === 'xai' && accounts && !acc?.signed_in && <div className="connect-row"><SignInGrok plan onDone={check} /><span className="hint">or paste a key below</span></div>}
            {provider === 'xai' && acc?.account && !acc.key && <p className="hint" style={{ margin: 0 }}>You’re using your Grok plan ({acc.account}), on xAI’s API.</p>}
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
