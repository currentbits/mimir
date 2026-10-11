import { useEffect, useRef, useState } from 'react'
import { invoke } from '../api'
import { Avatar, Empty, Icon } from '../ui'
import type { Bot, Chat } from '../types'

/** Keys from the browser → xdotool names. Printable characters are typed instead. */
const XKEY: Record<string, string> = {
  Enter: 'Return', Backspace: 'BackSpace', Tab: 'Tab', Escape: 'Escape', Delete: 'Delete', ArrowLeft: 'Left', ArrowRight: 'Right',
  ArrowUp: 'Up', ArrowDown: 'Down', Home: 'Home', End: 'End', PageUp: 'Prior', PageDown: 'Next', ' ': 'space',
}

/** Watch the bots' computer live, and take over: click, type, press keys and scroll on it yourself. */
export function ComputerView({ bots, chats, chatName }: { bots: Bot[]; chats: Chat[]; chatName: (c: Chat) => string }) {
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
