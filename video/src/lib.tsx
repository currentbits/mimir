// The film's clock, easing and the few pieces every scene shares: words that surface, typing, the
// bots' faces (the app's own face.css, moved by the frame instead of CSS animations), paper, ripples.
import React, { type CSSProperties, type ReactNode } from 'react'
import { AbsoluteFill, Easing, interpolate, random, Sequence, useCurrentFrame, useVideoConfig } from 'remotion'
import T from './timeline.json'
import G from './guide.json'
import A from './ad.json'
import { traits, type Mood } from '../../ui/src/face-core'
import { pigment } from '../../ui/src/format'
import { Icon } from '../../ui/src/ui'
import './film.css'
export { Icon, pigment }

// ---------------- palette & type (site/style.css and ui/src/index.css, dark) ----------------

export const C = {
  peat: '#14100B', deep: '#0A0705', foam: '#ECE9E1', ink2: '#CBC5B9', ink3: '#A8A195', gall: '#DFC19A',
  paper: '#F7F5EF', paper2: '#E9E5DB', pInk: '#1B1C1A', pInk2: '#55574F', pInk3: '#696960', pRule: 'rgba(27,28,26,0.12)',
  live: '#4CBF82', pending: '#E2A647', danger: '#F0675E',
  app: '#1A1A1C', surface: '#222225', sunken: '#2C2C31', text: '#EDEDEF', text2: '#A7A7AF', text3: '#7C7C84', line: 'rgba(255,255,255,0.08)',
}
export const F = {
  serif: 'Junicode, "Iowan Old Style", Georgia, serif',
  sans: '-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif',
  ui: 'ui-serif, "New York", Georgia, serif',
  mono: 'ui-monospace, "SF Mono", Menlo, monospace',
}
export const BOT = { ada: 1, researcher: 2, briefer: 3, reviewer: 4, watcher: 6 }

// ---------------- the beat clock ----------------

// The film, the guide and the ad share one clock: same tempo, and scene ids that don't clash.
const SCENES = { ...T.scenes, ...G.scenes, ...A.scenes }
for (const [name, X] of [['guide', G], ['ad', A]] as const)
  if (X.bpm !== T.bpm || Object.keys(X.scenes).some((k) => k in T.scenes || (X !== G && k in G.scenes))) throw new Error(`${name}.json: same bpm as timeline.json, and its own scene ids`)
export const BPM = T.bpm
export type SceneId = keyof typeof SCENES
export const fpb = (fps: number) => (fps * 60) / BPM
/** Beats since the current Sequence began (fractional). */
export function useBeat() {
  const frame = useCurrentFrame()
  const { fps } = useVideoConfig()
  return frame / fpb(fps)
}
/** A scene's cue beats by name (timeline.json or guide.json, shared with the score). */
export function cues(id: SceneId) {
  return Object.fromEntries(Object.entries(SCENES[id].cues).map(([k, v]) => [k, (v as unknown[])[0] as number])) as Record<string, number>
}
export const sceneBeats = (id: SceneId) => SCENES[id].len * 4

export const ease = {
  out: Easing.bezier(0.16, 1, 0.3, 1),
  io: Easing.bezier(0.65, 0, 0.35, 1),
  in: Easing.bezier(0.55, 0, 1, 0.45),
  back: Easing.bezier(0.34, 1.45, 0.64, 1),
  soft: Easing.bezier(0.45, 0, 0.55, 1),
  lin: Easing.linear,
}
/** 0→1 from beat `at` over `dur` beats. */
export const p = (t: number, at: number, dur = 1, e = ease.out) =>
  interpolate(t, [at, at + dur], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: e })
export const lerp = (a: number, b: number, k: number) => a + (b - a) * k
export const sec = (beats: number) => (beats * 60) / BPM
/** 1 for the moment a pointer presses at beat `at`. */
export const press = (t: number, at: number) => (t >= at - 0.12 && t < at + 0.18 ? 1 : 0)
/** A pointer gliding between points over beats, with a little arc. */
export const glide = (t: number, at: number, dur: number, from: [number, number], to: [number, number]) => {
  const k = p(t, at, dur, ease.io)
  return { x: lerp(from[0], to[0], k), y: lerp(from[1], to[1], k) - Math.sin(k * Math.PI) * 40 }
}

/** A scene on its bars. It drifts in slowly the whole time it's up, and sinks out over its last `exit` beats. */
export function Scene({ id, exit = 0.5, drift = 0.025, children }: { id: SceneId; exit?: number; drift?: number; children: ReactNode }) {
  const { fps } = useVideoConfig()
  const s = SCENES[id]
  return (
    <Sequence from={Math.round(s.at * 4 * fpb(fps))} durationInFrames={Math.round(s.len * 4 * fpb(fps))} name={id}>
      <Sink len={s.len * 4} exit={exit} drift={drift}>{children}</Sink>
    </Sequence>
  )
}
function Sink({ len, exit, drift, children }: { len: number; exit: number; drift: number; children: ReactNode }) {
  const t = useBeat()
  const k = exit ? p(t, len - exit, exit, ease.in) : 0
  return (
    <AbsoluteFill style={{ opacity: 1 - k, filter: k > 0.001 ? `blur(${k * 14}px)` : undefined, scale: 1 + (drift * t) / len - k * 0.03 }}>
      {children}
    </AbsoluteFill>
  )
}

// ---------------- type ----------------

export const headline: CSSProperties = { fontFamily: F.serif, fontWeight: 400, fontSize: 104, lineHeight: 1.04, letterSpacing: '-0.012em', color: C.foam, margin: 0 }
export const gloss: CSSProperties = { fontFamily: F.serif, fontStyle: 'italic', fontSize: 40, lineHeight: 1.3, color: C.gall, margin: 0 }
export const eyebrow: CSSProperties = { fontFamily: F.mono, fontSize: 22, letterSpacing: '0.2em', textTransform: 'uppercase', color: C.ink3 }

/** Words surface one after another: up, unblurred, in. `\n` breaks a line; `*word*` is italic in gall. */
export function Words({ text, at, step = 0.25, dur = 1.2, style, rise = 0.45 }: { text: string; at: number; step?: number; dur?: number; style?: CSSProperties; rise?: number }) {
  const t = useBeat()
  let i = 0
  return (
    <div style={style}>
      {text.split('\n').map((line, li) => (
        <span key={li} style={{ display: 'block' }}>
          {line.split(' ').map((w, wi) => {
            const k = p(t, at + i++ * step, dur)
            const em = w.startsWith('*')
            return (
              <React.Fragment key={wi}>
                {wi > 0 && ' '}
                <span style={{ display: 'inline-block', opacity: k, translate: `0 ${(1 - k) * rise}em`, filter: k < 0.999 ? `blur(${(1 - k) * 10}px)` : undefined, ...(em ? { fontStyle: 'italic', color: C.gall } : null) }}>
                  {w.replace(/\*/g, '')}
                </span>
              </React.Fragment>
            )
          })}
        </span>
      ))}
    </div>
  )
}

/** Something that rises into its slot at `at`. */
export function Rise({ at, dur = 1.2, y = 40, children, style }: { at: number; dur?: number; y?: number; children: ReactNode; style?: CSSProperties }) {
  const t = useBeat()
  const k = p(t, at, dur)
  return <div style={{ opacity: k, translate: `0 ${(1 - k) * y}px`, filter: k < 0.999 ? `blur(${(1 - k) * 8}px)` : undefined, ...style }}>{children}</div>
}

/** Typed by a person: uneven keystrokes, a caret while it's going. */
export function Typed({ text, at, dur, caret = true, seed = 'k' }: { text: string; at: number; dur: number; caret?: boolean; seed?: string }) {
  const t = useBeat()
  const gaps = [...text].map((ch, i) => (ch === ' ' ? 1.6 : 0.6 + random(`${seed}${i}`)))
  const total = gaps.reduce((a, b) => a + b, 0)
  let acc = 0, n = 0
  for (const g of gaps) { acc += g; if (at + (acc / total) * dur <= t) n++; else break }
  const typing = t >= at - 0.5 && t < at + dur + 1
  return (
    <>
      {text.slice(0, n)}
      {caret && typing && <span style={{ display: 'inline-block', width: 3, height: '1.05em', marginLeft: 3, verticalAlign: '-0.15em', background: 'currentColor', opacity: t < at || t > at + dur ? (Math.floor(t * 2) % 2 ? 0 : 0.8) : 0.8 }} />}
    </>
  )
}

/** A reply streaming in, a word at a time. */
export function Stream({ text, at, dur, style }: { text: string; at: number; dur: number; style?: CSSProperties }) {
  const t = useBeat()
  const words = text.split(' ')
  return (
    <span style={style}>
      {words.map((w, i) => {
        const k = p(t, at + (i / words.length) * dur, 0.35, ease.lin)
        return <React.Fragment key={i}>{i > 0 && ' '}<span style={{ opacity: k }}>{renderBold(w)}</span></React.Fragment>
      })}
    </span>
  )
}
const renderBold = (w: string) => (w.startsWith('**') ? <b style={{ fontWeight: 650 }}>{w.replace(/\*\*/g, '')}</b> : w)

/** Height that opens from nothing to its content (0fr → 1fr), so what's above moves up. */
export function Grow({ k, children }: { k: number; children: ReactNode }) {
  return (
    <div style={{ display: 'grid', gridTemplateRows: `${k}fr` }}>
      <div style={{ overflow: 'hidden', minHeight: 0 }}>{children}</div>
    </div>
  )
}

// ---------------- the bots' faces ----------------

const cycle = (s: number, period: number) => (((s / period) % 1) + 1) % 1
const hopAt = (u: number) => ({
  y: interpolate(u, [0, 0.2, 0.5, 0.8, 1], [0, 0, -0.28, 0, 0]),
  sx: interpolate(u, [0, 0.2, 0.5, 0.8, 1], [1, 1.12, 0.92, 1.1, 1]),
  sy: interpolate(u, [0, 0.2, 0.5, 0.8, 1], [1, 0.86, 1.1, 0.9, 1]),
})

/** A bot's face (ui/src/face.css), with its moods on the film's clock: `moods` is [[beat, mood], …]. */
export function Face({ id, size, moods = [[0, 'idle']], style }: { id: number; size: number; moods?: [number, Mood][]; style?: CSSProperties }) {
  const t = useBeat()
  let mood: Mood = moods[0][1], since = t - moods[0][0]
  for (const [b, m] of moods) if (t >= b) { mood = m; since = t - b }
  const s = sec(t) + id * 1.7, ms = sec(since)
  const tr = traits(id)

  // Blinks on its own clock (face-blink), never while eyes are shut.
  const period = 4.2 + (tr.h % 29) / 10
  const ph = cycle(s + (tr.h % 61) / 10, period)
  const blink = ph > 0.94 && ph < 0.98 ? 0.08 + (Math.abs(ph - 0.96) / 0.02) * 0.92 : 1
  const br = (1 - Math.cos((s / 3.8) * 2 * Math.PI)) / 2
  let bx = 1 - 0.015 * br, by = 1 + 0.035 * br, y = 0, rot = 0
  let ex = -50, ey = -50, eyeY = blink
  let mouth: CSSProperties = {}
  let ring = -1

  const g = cycle(s, 12)
  ex = interpolate(g, [0, 0.36, 0.4, 0.5, 0.54, 0.78, 0.82, 0.9, 1], [-50, -50, -64, -64, -50, -50, -38, -38, -50])
  ey = interpolate(g, [0, 0.78, 0.82, 0.9, 1], [-50, -50, -62, -62, -50])

  if (mood === 'thinking') { const k = Math.sin((s / 2.2) * 2 * Math.PI); bx = 1 + 0.03 * k; by = 1 - 0.025 * k; rot = 2.5 * k; ex = -50; ey = -50 }
  if (mood === 'searching') { ex = -50 + 16 * Math.sin((s / 1.8) * 2 * Math.PI); ey = -50; const k = (1 - Math.cos((s / 1.8) * 2 * Math.PI)) / 2; bx = 1 - 0.015 * k; by = 1 + 0.035 * k }
  if (mood === 'reading') { const r = cycle(s, 2.4); const line = Math.min(2, Math.floor(r * 3)); ex = -66 + 32 * cycle(r * 3, 1); ey = -56 + line * 10 }
  if (mood === 'writing') {
    const k = (1 - Math.cos((s / 0.84) * 2 * Math.PI)) / 2; y = -0.04 * k; bx = 1 - 0.02 * k; by = 1 + 0.03 * k; ex = -50; ey = -50
    const m = (1 - Math.cos((s / 0.6) * 2 * Math.PI)) / 2
    mouth = { opacity: 1, scale: `${0.7 + 0.15 * m} ${0.35 + 0.65 * m}` }
  }
  if (mood === 'asking') { const k = (1 - Math.cos((s / 1.5) * 2 * Math.PI)) / 2; bx = 1 + 0.04 * k; by = 1 + 0.05 * k; y = -0.02 * k; ex = -50; ey = -50 }
  if (mood === 'listening') { ex = -50; ey = -50; rot = 7 }
  if (mood === 'happy' || mood === 'remembering') { ex = -50; ey = -50; eyeY = 1 }
  if (mood === 'happy') {
    const u = ms / 0.62
    if (u < 2) { const h = hopAt(u % 1); y = h.y; bx = h.sx; by = h.sy }
    mouth = { opacity: 1, scale: 1 }
    ring = ms / 1.1
  }
  if (mood === 'remembering') { const k = Math.sin(cycle(s, 0.9) * Math.PI); y = 0.04 * k; bx = 1 + 0.03 * k; by = 1 - 0.04 * k; ring = cycle(ms, 1.1) }
  if (mood === 'attention') { const u = cycle(ms, 2.6) / 0.28; if (u < 1) { const h = hopAt(u); y = h.y * 0.8; bx = h.sx; by = h.sy } ex = -50; ey = -50 }

  return (
    <span style={{ position: 'relative', display: 'inline-block', flex: 'none', width: size, height: size, ['--s' as string]: `${size}px`, ['--pig' as string]: pigment(id), ...style }}>
      {ring >= 0 && ring < 1 && (
        <span style={{ position: 'absolute', left: '50%', bottom: '-2%', width: '104%', height: '24%', translate: '-50% 0', borderRadius: '50%', border: `${size * 0.022}px solid color-mix(in oklab, ${pigment(id)} 55%, transparent)`, opacity: 1 - ring, scale: 0.5 + 0.65 * ring }} />
      )}
      <span className="face" data-mood={mood} data-shape={tr.shape} data-eyes={tr.eyes} data-extra={tr.extra || undefined}>
        <span className="face-body" style={{ translate: `-50% ${y * size}px`, scale: `${bx} ${by}`, rotate: `${rot}deg` }}>
          <span className="face-look">
            <span className="face-eyes" style={{ translate: `${ex}% ${ey}%` }}>
              <i style={{ scale: `1 ${eyeY}` }} />
              <i style={{ scale: `1 ${eyeY}` }} />
            </span>
            <span className="face-mouth" style={mouth} />
          </span>
        </span>
      </span>
    </span>
  )
}

// ---------------- surfaces ----------------

const FIBRE = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='180' height='180'%3E%3Cfilter id='f'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix values='0 0 0 0 .32 0 0 0 0 .24 0 0 0 0 .12 0 0 0 .07 0'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23f)'/%3E%3C/svg%3E")`
export const ON_WATER = '0 30px 60px -20px rgba(0,0,0,0.85), 0 3px 8px rgba(0,0,0,0.35)'

/** Paper on the water: what you read sits on it as paper (as on the site). */
export function Paper({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return <div style={{ background: `${FIBRE}, ${C.paper}`, color: C.pInk, borderRadius: 16, boxShadow: ON_WATER, fontFamily: F.sans, ...style }}>{children}</div>
}
/** The app's own dark surface. */
export function Panel({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return <div style={{ background: C.surface, color: C.text, borderRadius: 22, boxShadow: `${ON_WATER}, inset 0 0 0 1px ${C.line}`, fontFamily: F.sans, overflow: 'hidden', ...style }}>{children}</div>
}
export function Chip({ icon, children, style }: { icon?: string; children: ReactNode; style?: CSSProperties }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 10, padding: '8px 16px 8px 13px', borderRadius: 99, background: 'rgba(255,255,255,0.06)', boxShadow: `inset 0 0 0 1px ${C.line}`, color: C.text2, fontSize: 22, whiteSpace: 'nowrap', ...style }}>
      {icon && <Icon name={icon} size={22} />}
      {children}
    </span>
  )
}

/** The pointer, drawn (macOS arrow). `press` 0..1 squeezes it on a click. */
export function Cursor({ x, y, press = 0, color = '#000', tag }: { x: number; y: number; press?: number; color?: string; tag?: ReactNode }) {
  return (
    <div style={{ position: 'absolute', left: x, top: y, scale: 1 - press * 0.12, transformOrigin: '0 0', filter: 'drop-shadow(0 4px 8px rgba(0,0,0,0.45))', zIndex: 20 }}>
      <svg width={40} height={48} viewBox="0 0 20 24">
        <path d="M1.5 1.5v17.2l4.3-4.1 2.9 6.8 3-1.3-2.9-6.6h6z" fill={color} stroke="#fff" strokeWidth={1.4} strokeLinejoin="round" />
      </svg>
      {tag && <div style={{ position: 'absolute', left: 30, top: 38, whiteSpace: 'nowrap' }}>{tag}</div>}
    </div>
  )
}

// ---------------- the well ----------------

/** Rings on the water from (x, y), starting at beat `at`. */
export function Ripple({ x, y, at, size = 900, rings = 3, color = C.foam, strength = 0.35, beats = 4 }: { x: number; y: number; at: number; size?: number; rings?: number; color?: string; strength?: number; beats?: number }) {
  const t = useBeat()
  if (t < at || t > at + beats + rings * 0.35) return null
  return (
    <>
      {Array.from({ length: rings }, (_, i) => {
        const k = p(t, at + i * 0.3, beats, ease.out)
        if (k <= 0 || k >= 1) return null
        const r = size * k
        return <div key={i} style={{ position: 'absolute', left: x - r / 2, top: y - r / 2, width: r, height: r, borderRadius: '50%', border: `${2.5 - i * 0.6}px solid ${color}`, opacity: strength * (1 - k) * (1 - i * 0.22) }} />
      })}
    </>
  )
}

/** The well behind everything: peat-dark water, light moving on it, old slips of paper at the bottom. */
export function Well() {
  const frame = useCurrentFrame()
  const { fps, width, height } = useVideoConfig()
  const s = frame / fps
  const slips = Array.from({ length: 16 }, (_, i) => ({
    x: random(`sx${i}`) * width, y: random(`sy${i}`) * height, w: 70 + random(`sw${i}`) * 90, r: (random(`sr${i}`) - 0.5) * 50,
    d: 0.4 + random(`sd${i}`) * 0.6,
  }))
  return (
    <AbsoluteFill style={{ background: `radial-gradient(ellipse 80% 75% at 50% 44%, #2A1D10 0%, #19120B 46%, ${C.deep} 100%)`, overflow: 'hidden' }}>
      {slips.map((sl, i) => (
        <div key={i} style={{
          position: 'absolute', left: sl.x + Math.sin(s * 0.13 * sl.d + i) * 30 * sl.d, top: sl.y + Math.cos(s * 0.11 * sl.d + i * 2) * 20 * sl.d,
          width: sl.w, height: sl.w * 0.62, rotate: `${sl.r + Math.sin(s * 0.1 + i) * 4}deg`, borderRadius: 4,
          background: '#E9E3D3', opacity: 0.025 + 0.035 * sl.d, filter: `blur(${4 + (1 - sl.d) * 6}px)`,
        }} />
      ))}
      <div style={{ position: 'absolute', inset: '-20%', opacity: 0.5,
        background: `radial-gradient(circle at ${42 + Math.sin(s * 0.09) * 8}% ${36 + Math.cos(s * 0.07) * 6}%, rgba(223,193,154,0.09), transparent 32%),
                     radial-gradient(circle at ${64 + Math.cos(s * 0.06) * 10}% ${62 + Math.sin(s * 0.08) * 8}%, rgba(223,193,154,0.06), transparent 30%)` }} />
    </AbsoluteFill>
  )
}

/** Film grain and the well's dark rim, over everything. */
export function Grain() {
  const frame = useCurrentFrame()
  return (
    <AbsoluteFill style={{ pointerEvents: 'none' }}>
      <AbsoluteFill style={{ background: 'radial-gradient(ellipse 75% 70% at 50% 46%, transparent 55%, rgba(5,3,2,0.6) 100%)' }} />
      <svg width="100%" height="100%" style={{ position: 'absolute', inset: 0, opacity: 0.07, mixBlendMode: 'screen' }}>
        <filter id="grain"><feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed={frame % 7} stitchTiles="stitch" /><feColorMatrix type="saturate" values="0" /></filter>
        <rect width="100%" height="100%" filter="url(#grain)" />
      </svg>
    </AbsoluteFill>
  )
}
