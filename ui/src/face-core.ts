// The parts of a bot's face that don't need React: its look, its mood, eyes that follow the pointer,
// and the same face as plain HTML. Shared by the app (face.tsx) and the website (site/main.js).
import './face.css'
import { lookOf, type Look } from './format'

export type Mood = 'idle' | 'listening' | 'thinking' | 'searching' | 'reading' | 'writing' | 'asking' | 'remembering'
  | 'happy' | 'sad' | 'surprised' | 'asleep' | 'attention'

/** A reply in progress, as a mood: the step it's on, or writing once text arrives. */
export function moodOf(step: string | null, draft: string): Mood {
  if (step) {
    if (/your OK/.test(step)) return 'asking'
    if (/^(Saving to|Forgetting|Writing it down)/.test(step)) return 'remembering'
    if (/^(Searching|Looking)/.test(step)) return 'searching'
    if (/^(Reading|Using the (browser|computer))/.test(step)) return 'reading'
    return 'thinking'
  }
  return draft ? 'writing' : 'thinking'
}

export const SHAPES = ['pebble', 'dome', 'bean', 'tall'] as const
export const EYES = ['oval', 'round', 'wide', 'bead'] as const
/** The ones a bot can get by default come first; antenna and freckles are only by choice. */
export const EXTRAS = ['cheeks', 'sprout', '', 'antenna', 'freckles'] as const
const pick = <T extends string>(list: readonly T[], v: string | undefined, d: T): T => (list as readonly string[]).includes(v ?? '\u0000') ? v as T : d
/** Bot id → its look: what you chose, else shape, eyes and detail cycling at different rates, so neighbouring bots
 *  never match. */
export function traits(id: number, look: Look | undefined = lookOf(id)) {
  const n = Math.max(id, 1) - 1
  const h = (Math.imul(id ^ 0x9e3779b9, 0x85ebca6b) >>> 0) % 1000 // blink and glance timing, so faces don't move in step
  return { shape: pick(SHAPES, look?.shape, SHAPES[n % 4]), eyes: pick(EYES, look?.eyes, EYES[(n * 3) % 4]), extra: pick(EXTRAS, look?.extra, EXTRAS[n % 3]), h }
}
/** Each face blinks and glances on its own clock. */
export const faceTiming = (h: number) => ({ '--blink': `${4.2 + (h % 29) / 10}s`, '--blink-at': `-${(h % 61) / 10}s`, '--glance-at': `-${(h % 97) / 10}s` })

/** A face as HTML, for pages without React. Put it in an element that sets --pig and --s. */
export function faceHTML(id: number, mood: Mood = 'idle') {
  const t = traits(id), style = Object.entries(faceTiming(t.h)).map(([k, v]) => `${k}:${v}`).join(';')
  return `<span class="face" aria-hidden="true" style="${style}" data-mood="${mood}" data-shape="${t.shape}" data-eyes="${t.eyes}"${t.extra ? ` data-extra="${t.extra}"` : ''}>`
    + '<span class="face-body"><span class="face-look"><span class="face-eyes"><i></i><i></i></span><span class="face-mouth"></span></span></span></span>'
}

// ---------------- eyes that follow the pointer ----------------
// One listener for every face that's watching; each frame aims their eyes at the pointer, and lets
// them wander again when it's far away.
const watching = new Set<HTMLElement>()
let px = 0, py = 0, frame = 0
const aim = () => {
  frame = 0
  for (const el of watching) {
    const r = el.getBoundingClientRect(), dx = px - (r.left + r.width / 2), dy = py - (r.top + r.height / 2)
    const near = Math.hypot(dx, dy) < 520
    el.toggleAttribute('data-looking', near)
    el.style.setProperty('--lx', near ? Math.max(-1, Math.min(1, dx / 180)).toFixed(3) : '0')
    el.style.setProperty('--ly', near ? Math.max(-1, Math.min(1, dy / 180)).toFixed(3) : '0')
  }
}
const onMove = (e: PointerEvent) => { px = e.clientX; py = e.clientY; if (!frame) frame = requestAnimationFrame(aim) }
/** Make a .face follow the pointer; returns the way to stop. */
export function watch(el: HTMLElement) {
  if (!watching.size) addEventListener('pointermove', onMove, { passive: true })
  watching.add(el)
  return () => { watching.delete(el); if (!watching.size) removeEventListener('pointermove', onMove) }
}
