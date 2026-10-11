// The quick-ask panel (desktop): summoned with a global shortcut from any app, Spotlight-style.
// Same built-in chat as the main window, so every exchange is also there afterwards.
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { inDesktop, invoke, localInvoke } from './api'
import { Markdown } from './markdown'
import { Avatar, Icon, talkLabel, useHoldToTalk } from './ui'
import { pigment, setLooks } from './format'

type Bot = { id: number; name: string; look?: string }
const WIDTH = 680

export function QuickAsk() {
  const [bots, setBots] = useState<Bot[]>([])
  const [botId, setBotId] = useState(0)
  const [text, setText] = useState('')
  const [asked, setAsked] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const [draft, setDraft] = useState('')
  const [answer, setAnswer] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const panel = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const win = getCurrentWindow()
  const talk = useHoldToTalk(w => setText(t => t.trim() ? `${t.trimEnd()} ${w}` : w), setErr)

  useEffect(() => {
    // The bots as they are now (renamed, added or deleted in the main window since); the chosen one stays if it's still there.
    const load = () => invoke<Bot[]>('bots').then(b => { setLooks(b); setBots(b); setBotId(id => b.some(x => x.id === id) ? id : b[0]?.id ?? id) }, () => {})
    load()
    // Each summon: focus the field and let the panel materialize again.
    const focus = () => {
      load()
      input.current?.focus(); input.current?.select()
      const el = panel.current
      if (el) { el.classList.remove('summon'); void el.offsetWidth; el.classList.add('summon') }
    }
    window.addEventListener('focus', focus)
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') win.hide() }
    window.addEventListener('keydown', esc)
    return () => { window.removeEventListener('focus', focus); window.removeEventListener('keydown', esc) }
  }, [win])

  // The native window follows the panel's height.
  useLayoutEffect(() => {
    const el = panel.current
    if (!el) return
    const fit = () => win.setSize(new LogicalSize(WIDTH, Math.ceil(el.getBoundingClientRect().height))).catch(() => {})
    const ro = new ResizeObserver(fit)
    ro.observe(el)
    fit()
    return () => ro.disconnect()
  }, [win])

  useEffect(() => {
    if (!pending) return
    const t = setInterval(() => invoke<string | null>('draft', { botId, chatId: 0 }).then(d => setDraft(d ?? ''), () => {}), 120)
    return () => { clearInterval(t); setDraft('') }
  }, [pending, botId])

  const send = () => {
    const t = text.trim()
    if (!t || pending) return
    setAsked(t); setText(''); setAnswer(null); setErr(null); setPending(true)
    invoke<string>('send_local', { botId, text: t }).then(setAnswer, e => setErr(String(e))).finally(() => setPending(false))
  }
  const bot = bots.find(b => b.id === botId) ?? { id: botId, name: 'Mimir' }
  const shown = answer ?? draft

  return (
    <div className="quick" ref={panel} style={{ '--pig': pigment(bot.id) } as React.CSSProperties}>
      <form className="quick-bar" onSubmit={e => { e.preventDefault(); send() }}>
        <span className="bot-pick" title={bots.length > 1 ? 'Choose a bot' : bot.name}>
          <Avatar id={bot.id} name={bot.name} size={30} alive beat={text.length}
            mood={pending ? (draft ? 'writing' : 'thinking') : talk.state !== 'idle' || text.trim() ? 'listening' : err ? 'sad' : 'idle'} />
          {bots.length > 1 && (
            <select value={botId} onChange={e => setBotId(Number(e.target.value))} disabled={pending} aria-label="Bot to ask">
              {bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
            </select>
          )}
        </span>
        <input ref={input} autoFocus placeholder={pending ? `${bot.name} is answering…` : talk.state === 'listening' ? 'Listening…' : `Ask ${bot.name}`} value={text} onChange={e => setText(e.target.value)} aria-label={`Ask ${bot.name}`} {...talk.keys} />
        {talk.state === 'listening' ? <kbd>listening…</kbd> : talk.state === 'writing' ? <kbd>writing…</kbd> : text.trim() ? <kbd>⏎</kbd> : inDesktop ? <kbd title="Hold to dictate">hold {talkLabel} to talk</kbd> : null}
      </form>
      {(asked || err) && (
        <div className="quick-body">
          {asked && <p className="quick-q">{asked}</p>}
          {err ? <p className="error">{err}</p>
            : shown ? <div className="msg-body"><Markdown text={shown} caret={pending} /></div>
            : <span className="typing" aria-label="Thinking"><i /><i /><i /></span>}
          <div className="quick-foot">
            <button type="button" className="btn btn-sm btn-ghost" onClick={() => localInvoke('show_main').catch(e => setErr(String(e)))}><Icon name="chat" size={13} />Open in Mimir</button>
            <span>esc to close</span>
          </div>
        </div>
      )}
    </div>
  )
}
