import { createContext, useCallback, useEffect, useRef, useState } from 'react'
import { inDesktop, keystore, remoteServer } from './api'
import { EXTRAS, EYES, SHAPES, traits } from './face-core'
import { fullTime, pigment, PIGMENTS, schedule, type Look } from './format'
import { Markdown } from './markdown'
import { Avatar, Icon, type Presence } from './ui'
import type { Rehearsed, Routine, Run, Status } from './types'

const any = <T,>(list: readonly T[]) => list[Math.floor(Math.random() * list.length)]

/** A fresh creature: what a new bot starts as (change it, or shuffle again). */
export const randomLook = (): Look => ({ shape: any(SHAPES), eyes: any(EYES), extra: any(EXTRAS), color: any(PIGMENTS) })

const LOOK_NAMES: Record<string, string> = {
  pebble: 'Pebble', dome: 'Dome', bean: 'Bean', tall: 'Tall', oval: 'Oval', round: 'Round', wide: 'Wide apart', bead: 'Beady',
  cheeks: 'Rosy cheeks', sprout: 'Sprout', '': 'Nothing', antenna: 'Antenna', freckles: 'Freckles',
}

/** Build a bot's creature: colour, shape, eyes and one detail, each shown on the creature itself. */
export function LookPicker({ id, look, onChange }: { id: number; look: Look; onChange: (l: Look) => void }) {
  const t = traits(id, look)
  const now: Look = { shape: t.shape, eyes: t.eyes, extra: t.extra, color: pigment(id, look) }
  const set = (p: Look) => onChange({ ...now, ...p })
  const Opts = ({ part, list }: { part: 'shape' | 'eyes' | 'extra'; list: readonly string[] }) => (
    <div className="look-opts" role="radiogroup" aria-label={part === 'extra' ? 'Detail' : part === 'eyes' ? 'Eyes' : 'Shape'}>
      {list.map(v => (
        <button key={v} type="button" role="radio" aria-checked={now[part] === v} className={'look-opt' + (now[part] === v ? ' on' : '')} title={LOOK_NAMES[v]} onClick={() => set({ [part]: v })}>
          <Avatar id={id} name="" size={34} still look={{ ...now, [part]: v }} /><small>{LOOK_NAMES[v]}</small>
        </button>
      ))}
    </div>
  )
  return (
    <div className="look-picker">
      <div className="look-row"><span>Colour</span>
        <div className="swatches" role="radiogroup" aria-label="Colour">{PIGMENTS.map(c => (
          <button key={c} type="button" role="radio" aria-checked={now.color === c} aria-label={`Colour ${c}`} className={'swatch' + (now.color === c ? ' on' : '')} style={{ background: c }} onClick={() => set({ color: c })} />
        ))}</div></div>
      <div className="look-row"><span>Shape</span><Opts part="shape" list={SHAPES} /></div>
      <div className="look-row"><span>Eyes</span><Opts part="eyes" list={EYES} /></div>
      <div className="look-row"><span>Detail</span><Opts part="extra" list={EXTRAS} /></div>
      <div className="look-row"><span /><button type="button" className="btn btn-sm btn-ghost" onClick={() => onChange(randomLook())}><Icon name="retry" size={13} />Surprise me</button></div>
    </div>
  )
}

/** A routine, as saved, right where it was made: what and when, and run it now, pause or delete it. */
/** A command name for a skill, as the bots read it: "Book padel" → "book-padel". */
export const skillSlug = (name: string) => name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '')

/** What a rehearsal found, and what it would have done (nothing was sent or changed). */
export function RehearsalResult({ r, onClose }: { r: Rehearsed; onClose: () => void }) {
  return (
    <div className="rehearsal">
      <div className="rehearsal-head"><b>Rehearsal</b><span className="hint grow">Nothing was sent or changed.</span><button className="btn btn-ghost btn-sm" onClick={onClose}>Close</button></div>
      {r.would.length > 0
        ? <div><span className="field-label">It would have</span><ul>{r.would.map((w, i) => <li key={i} className="selectable">{w}</li>)}</ul></div>
        : <span className="hint">It wouldn’t change or send anything.</span>}
      {r.looked.length > 0 && <span className="hint">It looked at: {r.looked.join(' · ')}</span>}
      <div><span className="field-label">What it would say</span><div className="selectable"><Markdown text={r.reply.trim() || 'Nothing needed your attention.'} /></div></div>
    </div>
  )
}

/** When a routine runs, in words: its schedule, or the one time a one-off (a reminder) runs. */
export const when = (r: Routine) => r.once && !r.cron ? `${r.followup ? 'Follow-up' : 'Once'} · ${fullTime(r.next_fire)}` : schedule(r.cron)

export const RUN_DOT: Record<Run['status'], string> = { ok: 'live', error: 'err', missed: 'missed', running: 'wait', unchanged: 'same' }

export const LOCAL = 0

/** The household's memories (shared with the guests you add) live in a chat of their own. */
export const HOUSEHOLD = -2

/** The incognito chat (built-in chat only): messages stay in the window; nothing is stored. */
export const INCOGNITO = -1

/** Where tokens and keys are kept. */
export const SECRETS_AT = inDesktop && !remoteServer() ? keystore : "your server's private database"

export const CHANNEL: Record<string, { name: string; icon: string }> = {
  telegram: { name: 'Telegram', icon: 'telegram' }, slack: { name: 'Slack', icon: 'slack' },
  discord: { name: 'Discord', icon: 'discord' }, signal: { name: 'Signal', icon: 'signal' }, whatsapp: { name: 'WhatsApp', icon: 'whatsapp' }, imessage: { name: 'iMessage', icon: 'imessage' }, household: { name: 'Shared with your household', icon: 'home' }, api: { name: 'Chat API', icon: 'plug' }, local: { name: 'Built-in chat', icon: inDesktop && !remoteServer() ? 'laptop' : 'globe' },
}

export const online = (s?: Status) => s?.channels.filter(c => c.connected) ?? []

export const presenceOf = (s?: Status): Presence => !s ? 'off' : online(s).length ? 'live' : s.error || s.channels.some(c => c.error) ? 'err' : s.channels.length ? 'wait' : 'off'

/** Bot id → bot, provided by Main. */
export const Bots = createContext<(id: number) => { id: number; name: string }>(id => ({ id, name: `Bot ${id}` }))

/** Load once and on demand; with `ms`, again `ms` after each answer. Nothing loads while the page is hidden;
 *  it loads again when it shows. */
export function useLoad<T>(load: () => Promise<T>, deps: unknown[], ms = 0) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  // Each change of what's loaded (another chat, another bot) starts a new generation: an answer for the one before,
  // still on its way, is dropped instead of landing over the new one.
  const gen = useRef(0)
  const mounted = useRef(false)
  // The last answer as JSON: the same answer again keeps the old state, so nothing redraws.
  const last = useRef('')
  const reload = useCallback(() => {
    const g = gen.current
    return load().then(d => {
      if (g !== gen.current) return
      const j = JSON.stringify(d)
      if (j !== last.current) { last.current = j; setData(d) }
      setError(null)
    }, e => { if (g === gen.current) setError(String(e)) })
  }, deps) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    // What was shown for the one before goes too (a banner, a list), rather than standing in until the new one loads.
    if (mounted.current) { gen.current++; last.current = ''; setData(null); setError(null) }
    mounted.current = true
    // One load at a time: the next is timed from the answer, so a slow server never gets them stacked up.
    let t = 0, busy = false, done = false
    const tick = () => {
      clearTimeout(t)
      if (busy || done || document.hidden) return
      busy = true
      reload().finally(() => { busy = false; if (!done && ms > 0) t = window.setTimeout(tick, ms) })
    }
    tick()
    document.addEventListener('visibilitychange', tick)
    return () => { done = true; clearTimeout(t); document.removeEventListener('visibilitychange', tick) }
  }, [reload, ms])
  return { data, error, reload }
}
