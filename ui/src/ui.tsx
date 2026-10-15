// Small shared UI pieces: icons, bot faces, controls, and human-friendly time/schedule text.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { Face, type Mood } from './face'
import { pigment, type Look } from './format'
import { inDesktop, localInvoke, platform } from './api'

// ---------------- icons (24px grid, stroked) ----------------

const PATHS: Record<string, string> = {
  chat: 'M20.5 11.5a8 8 0 0 1-11.7 7.1L4 20l1.3-4.4A8 8 0 1 1 20.5 11.5Z',
  memory: 'M12 6.5C10.5 5 8.3 4.5 4 4.5v14c4.3 0 6.5.5 8 2 1.5-1.5 3.7-2 8-2v-14c-4.3 0-6.5.5-8 2Zm0 0v14',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 4.5V12l3 2',
  activity: 'M3 12h4l3-7.5 4 15 3-7.5h4',
  settings: 'M4 7h9m4 0h3M4 17h4m4 0h8M15 5v4M10 15v4',
  plus: 'M12 5v14M5 12h14',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14Zm9 16-4-4',
  send: 'M12 19V6m-6 6 6-6 6 6',
  check: 'm5 12.5 4.5 4.5L19 7',
  x: 'M6.5 6.5l11 11m0-11-11 11',
  trash: 'M4.5 7h15M10 11v6m4-6v6M6.5 7l.8 11.5A2 2 0 0 0 9.3 20.5h5.4a2 2 0 0 0 2-2L17.5 7M9.5 7V4.5h5V7',
  pencil: 'M4.5 19.5h4l10-10-4-4-10 10v4Zm8-12 4 4',
  moon: 'M19.5 14.5A7.5 7.5 0 1 1 9.5 4.5a6 6 0 0 0 10 10Z',
  laptop: 'M5 6.5a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1V15H5V6.5ZM3 18.5h18',
  globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm-9 9h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9Z',
  telegram: 'm20.5 4.5-17 7 6 2.2m11-9.2-3 15-8-5.8m11-9.2-11 9.2v5l3-3.4',
  slack: 'M5 9.5h14M5 14.5h14M10 4.5l-1.5 15M15.5 4.5 14 19.5',
  signal: 'M12 3.5a8.5 8.5 0 0 0-7.4 12.7L3.5 20.5l4.3-1.1A8.5 8.5 0 1 0 12 3.5Z',
  discord: 'M7 7.5c3.3-1.3 6.7-1.3 10 0l1.5 2.5c1 2.2 1.3 4.5 1 7-1.5 1-3 1.7-4.5 2l-1-2m-4 0-1 2c-1.5-.3-3-1-4.5-2-.3-2.5 0-4.8 1-7L7 7.5ZM9.5 13.5h.01M14.5 13.5h.01',
  key: 'M14.5 4a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM10.5 13.5 4 20m2-2 2 2m-.5-4.5 2 2',
  shield: 'M12 3.5 19 6v5.5c0 4.5-3 7.6-7 9-4-1.4-7-4.5-7-9V6l7-2.5Z',
  server: 'M5 4.5h14v6H5zM5 13.5h14v6H5zM8.5 7.5h.01M8.5 16.5h.01',
  history: 'M4 12a8 8 0 1 0 2.3-5.7L4 8.5M4 4v4.5h4.5M12 8v4l2.5 1.5',
  wand: 'm4.5 19.5 10-10m2-5 .7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7.7-1.8Zm-4 0v1.5M19.5 11H21',
  chevron: 'm9 6 6 6-6 6',
  down: 'm6 9 6 6 6-6',
  browser: 'M3.5 5.5h17v13h-17zM3.5 9.5h17M6.5 7.5h.01M9 7.5h.01',
  alert: 'M12 9v4.5m0 3h.01M10.3 4.2 2.8 17.5A2 2 0 0 0 4.5 20.5h15a2 2 0 0 0 1.7-3L13.7 4.2a2 2 0 0 0-3.4 0Z',
  bolt: 'M13 3.5 5.5 13H12l-1 7.5L18.5 11H12l1-7.5Z',
  logout: 'M14.5 4.5h4v15h-4M10 8l-4 4 4 4m-4-4h11',
  home: 'M4 10.5 12 4l8 6.5V20h-5.5v-6h-5v6H4Z',
  user: 'M12 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8Zm-7 16c.8-3.5 3.6-5.5 7-5.5s6.2 2 7 5.5',
  plug: 'M9 3.5V8m6-4.5V8M6.5 8h11v2.5a5.5 5.5 0 0 1-11 0V8Zm5.5 8v4.5',
  inbox: 'M3.5 13.5h4.5l1.5 2.5h5l1.5-2.5h4.5M3.5 13.5 6.2 5.6A1.5 1.5 0 0 1 7.6 4.5h8.8a1.5 1.5 0 0 1 1.4 1.1l2.7 7.9V18a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 18v-4.5Z',
  pin: 'M9 4.5h6l-1 5.5 3 3v1.5H7V13l3-3-1-5.5ZM12 15v5',
  mic: 'M12 3.5a3 3 0 0 0-3 3V12a3 3 0 0 0 6 0V6.5a3 3 0 0 0-3-3ZM6 11.5a6 6 0 0 0 12 0M12 17.5v3',
  archive: 'M4 5h16v4H4zM5.5 9v10h13V9M10 13h4',
  file: 'M14 3H7a1.5 1.5 0 0 0-1.5 1.5v15A1.5 1.5 0 0 0 7 21h10a1.5 1.5 0 0 0 1.5-1.5V7.5L14 3Zm0 0v4.5h4.5M9 13h6M9 16.5h6',
  incognito: 'M3 10.5h18M6 10.5l1.5-5.5h9l1.5 5.5M7.5 18.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Zm9 0a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5ZM10 16h4',
  copy: 'M8.5 8.5V5A1.5 1.5 0 0 1 10 3.5h9A1.5 1.5 0 0 1 20.5 5v9a1.5 1.5 0 0 1-1.5 1.5h-3.5M5 8.5h9a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 19v-9A1.5 1.5 0 0 1 5 8.5Z',
  pause: 'M9 5.5v13m6-13v13',
  play: 'M7.5 5.5v13l11-6.5-11-6.5Z',
  retry:'M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 9m0-4.5V9H9',
}

export function Icon({ name, size = 16, className }: { name: keyof typeof PATHS | string; size?: number; className?: string }) {
  return (
    <svg className={'icon ' + (className ?? '')} width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name] ?? ''} />
    </svg>
  )
}

// ---------------- bots: a pigment each ----------------

export type Presence = 'live' | 'wait' | 'err' | 'off'

/** A bot's face (face.tsx). Without a mood it shows its connection: upset when a messaging app fails. */
/** `look`: a look being edited (a preview); otherwise the bot's own. */
export function Avatar({ id, size = 28, presence, mood, alive, still, beat, look }: {
  id: number; name: string; size?: number; presence?: Presence; mood?: Mood; alive?: boolean; still?: boolean; beat?: number; look?: Look
}) {
  return (
    <span className="avatar" style={{ '--pig': look ? pigment(id, look) : pigment(id), '--s': `${size}px`, width: size, height: size } as React.CSSProperties}>
      <Face id={id} mood={mood ?? (presence === 'err' ? 'sad' : 'idle')} alive={alive} still={still} beat={beat} look={look} />
      {presence && presence !== 'off' && <i className={'presence ' + presence} />}
    </span>
  )
}

// ---------------- controls ----------------

export function Switch({ checked, onChange, disabled, label }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean; label: string }) {
  return <input type="checkbox" role="switch" className="switch" aria-label={label} checked={checked} disabled={disabled} onChange={e => onChange(e.target.checked)} />
}

export function Segmented<T extends string>({ value, options, onChange, label, size }: {
  value: T; options: { id: T; label: ReactNode; icon?: string; hint?: string }[]; onChange: (v: T) => void; label: string; size?: 'sm'
}) {
  // One thumb glides to the selected option. First placement and resizes jump; changes slide.
  const box = useRef<HTMLDivElement>(null)
  const [placed, setPlaced] = useState(false)
  const place = (animate: boolean) => {
    const el = box.current
    const on = el?.querySelector<HTMLElement>('button.on')
    if (!el || !on) return
    if (!animate) el.classList.add('measuring')
    el.style.setProperty('--x', `${on.offsetLeft}px`)
    el.style.setProperty('--w', `${on.offsetWidth}px`)
    if (!animate) requestAnimationFrame(() => el.classList.remove('measuring'))
  }
  useLayoutEffect(() => { place(placed); setPlaced(true) }, [value, options.length]) // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    let first = true
    const ro = new ResizeObserver(() => { if (first) first = false; else place(false) })
    if (box.current) ro.observe(box.current)
    return () => ro.disconnect()
  }, []) // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div ref={box} className={'segmented' + (size ? ' ' + size : '') + (placed ? ' glide' : '')} role="tablist" aria-label={label}>
      {placed && <span className="thumb" aria-hidden="true" />}
      {options.map(o => (
        <button key={o.id} type="button" role="tab" aria-selected={value === o.id} className={value === o.id ? 'on' : ''} onClick={() => onChange(o.id)} title={o.hint}>
          {o.icon && <Icon name={o.icon} size={14} />}{o.label}
        </button>
      ))}
    </div>
  )
}

/** A row in an inset grouped list (System Settings style). */
export function Row({ title, desc, children, icon }: { title: ReactNode; desc?: ReactNode; children?: ReactNode; icon?: ReactNode }) {
  return (
    <div className="row">
      {icon && <span className="row-icon">{icon}</span>}
      <div className="row-main"><div className="row-title">{title}</div>{desc && <div className="row-desc">{desc}</div>}</div>
      {children && <div className="row-ctl">{children}</div>}
    </div>
  )
}

export function Group({ title, note, children, action }: { title?: ReactNode; note?: ReactNode; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="group-wrap">
      {(title || action) && <div className="group-head"><h3>{title}</h3>{action}</div>}
      <div className="group">{children}</div>
      {note && <p className="group-note">{note}</p>}
    </section>
  )
}

/** An empty state: an icon by name, or anything else (a bot's face) in its place. */
export function Empty({ icon, title, children, actions }: { icon: string | ReactNode; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="empty">
      {typeof icon === 'string' ? <span className="empty-icon"><Icon name={icon} size={22} /></span> : icon}
      <h3>{title}</h3>
      {children && <p>{children}</p>}
      {actions && <div className="empty-actions">{actions}</div>}
    </div>
  )
}

// ---------------- asking you: in the app, not the browser's own dialogs ----------------
// The desktop app's webview has no confirm() or prompt(): they return at once, unanswered. So every "Delete …?" goes
// through this one dialog, drawn by <AskHost /> (in the app's root).

type Asking = { text: string; ok: string; danger: boolean; value?: string; notice?: boolean; done: (v: string | null) => void }
let show: ((a: Asking) => void) | null = null

/** "Delete this routine?": true when you press the button, false for Cancel, Escape or a click beside it. */
export function ask(text: string, ok = 'OK', danger = false): Promise<boolean> {
  return new Promise(res => (show ? show({ text, ok, danger, done: v => res(v !== null) }) : res(window.confirm(text))))
}

/** Something went wrong with what you just pressed ("Couldn't copy: …"): said in the app, with OK. */
export function tell(text: string): Promise<void> {
  return new Promise(res => (show ? show({ text, ok: 'OK', danger: false, notice: true, done: () => res() }) : (window.alert(text), res())))
}

/** A short answer ("Who is Speaker 2?"); null when cancelled. */
export function askText(text: string, value = '', ok = 'Save'): Promise<string | null> {
  return new Promise(res => (show ? show({ text, ok, danger: false, value, done: res }) : res(window.prompt(text, value))))
}

export function AskHost() {
  const [a, setA] = useState<Asking | null>(null)
  const [val, setVal] = useState('')
  useEffect(() => {
    // A second question while one is open replaces it; the first counts as cancelled (its caller isn't left waiting).
    show = x => { setVal(x.value ?? ''); setA(old => { old?.done(null); return x }) }
    return () => { show = null }
  }, [])
  if (!a) return null
  const close = (v: string | null) => { setA(null); a.done(v) }
  return (
    <div className="ask-backdrop" onMouseDown={e => e.target === e.currentTarget && close(null)}>
      <form className="ask-box" role="alertdialog" aria-modal="true" aria-label={a.text}
        onSubmit={e => { e.preventDefault(); close(a.value !== undefined ? val : '') }} onKeyDown={e => e.key === 'Escape' && close(null)}>
        <p>{a.text}</p>
        {a.value !== undefined && <input className="input" autoFocus value={val} onChange={e => setVal(e.target.value)} aria-label={a.text} />}
        <div className="ask-actions">
          {!a.notice && <button type="button" className="btn btn-ghost" onClick={() => close(null)}>Cancel</button>}
          <button className={'btn ' + (a.danger ? 'btn-danger-solid' : 'btn-primary')} autoFocus={a.value === undefined}>{a.ok}</button>
        </div>
      </form>
    </div>
  )
}

// ---------------- push-to-talk ----------------

/** The key to hold for push-to-talk: Option on a Mac, Ctrl elsewhere (Alt alone would open Windows' window menu). */
const TALK_KEY = platform === 'mac' ? 'Alt' : 'Control'
export const talkLabel = platform === 'mac' ? '⌥' : 'Ctrl'

/** Push-to-talk (desktop): hold the key on its own and speak; on release, whisper.cpp on this computer writes the words
 *  into the box. Nothing is sent. The key with another one (⌥E for é, Ctrl+C) types as usual and drops the recording. */
export function useHoldToTalk(add: (words: string) => void, fail: (e: string) => void) {
  const [state, setState] = useState<'idle' | 'listening' | 'writing'>('idle')
  const timer = useRef<number | undefined>(undefined)
  const started = useRef<Promise<boolean> | null>(null)
  const stop = (keep: boolean) => {
    clearTimeout(timer.current)
    timer.current = undefined
    const s = started.current
    if (!s) return
    started.current = null
    setState(keep ? 'writing' : 'idle')
    // After the start has gone through, or the microphone would stay open.
    s.then(ok => ok ? localInvoke<string>('dictate', { action: 'stop' }) : '')
      .then(w => { if (keep && w.trim()) add(w.trim()) }, e => { if (keep) fail(String(e)) })
      .finally(() => setState('idle'))
  }
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!inDesktop) return
    if (e.key !== TALK_KEY) { stop(false); return }
    if (e.repeat || timer.current || started.current) return
    // A short hold, so a quick tap or a shortcut doesn't start the microphone.
    timer.current = window.setTimeout(() => {
      timer.current = undefined
      setState('listening')
      started.current = localInvoke('dictate', { action: 'start' }).then(() => true, err => { fail(String(err)); return false })
    }, 250)
  }
  const onKeyUp = (e: React.KeyboardEvent) => { if (e.key === TALK_KEY) stop(true) }
  return { state, keys: { onKeyDown, onKeyUp, onBlur: () => stop(false) } }
}
