// The myth, the mark, and the mark again at the end.
import { AbsoluteFill } from 'remotion'
import { C, F, cues, ease, gloss, Grow, headline, p, Ripple, useBeat, Words } from '../lib'

// The mark (crates/mimir-desktop/icons/icon.svg without its tile): ᛗ inside a speech bubble that is also the well.
const BUBBLE = 'M389 728A270 270 0 1 0 278 617L244 750Z'
const RUNE = ['M404 360V636', 'M624 360V636', 'M404 360L624 536', 'M624 360L404 536']

export function Mark({ size, id, draw = 1, fill = 1, rune = [1, 1, 1, 1] }: { size: number; id: string; draw?: number; fill?: number; rune?: number[] }) {
  return (
    <svg width={size} height={size} viewBox="190 170 652 652" style={{ overflow: 'visible' }}>
      <defs>
        <radialGradient id={`${id}w`} cx="0.5" cy="0.62" r="0.62"><stop offset="0" stopColor="#4A3017" /><stop offset="0.6" stopColor="#1B130A" /><stop offset="1" stopColor="#0A0705" /></radialGradient>
        <linearGradient id={`${id}r`} x1="0" y1="352" x2="0" y2="640" gradientUnits="userSpaceOnUse"><stop offset="0" stopColor="#F3EFE7" /><stop offset="0.62" stopColor="#ECE4D4" /><stop offset="1" stopColor="#D2A86C" /></linearGradient>
      </defs>
      <path d={BUBBLE} fill={`url(#${id}w)`} fillOpacity={fill} stroke="#ECE9E1" strokeWidth={40} strokeLinejoin="round" strokeLinecap="round"
        pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - draw} opacity={draw > 0 ? 1 : 0} />
      {RUNE.map((d, i) => (
        <path key={i} d={d} fill="none" stroke={`url(#${id}r)`} strokeWidth={56} strokeLinecap="round" pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - rune[i]} opacity={rune[i] > 0.01 ? 1 : 0} />
      ))}
    </svg>
  )
}

export function Cold() {
  const t = useBeat()
  const c = cues('cold')
  const out = p(t, c.out1, 0.9, ease.in)
  const splash = (at: number) => { const k = p(t, at, 0.5, ease.out); return k > 0 && k < 1 ? 1 - k : 0 }
  return (
    <AbsoluteFill>
      <Ripple x={960} y={560} at={c.drop} size={1500} rings={4} strength={0.32} beats={7} />
      <Ripple x={960} y={540} at={c.free} size={1700} rings={4} strength={0.36} beats={6} />
      {[c.drop, c.free].map((at, i) => (
        <div key={i} style={{ position: 'absolute', left: 960 - 40, top: (i ? 540 : 560) - 40, width: 80, height: 80, borderRadius: '50%', background: 'radial-gradient(circle, rgba(236,233,225,0.55), transparent 65%)', opacity: splash(at), scale: 0.4 + (1 - splash(at)) * 1.4 }} />
      ))}
      <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center', opacity: 1 - out, filter: out > 0 ? `blur(${out * 14}px)` : undefined, translate: `0 ${-out * 40}px` }}>
        <Words text="Odin gave an eye" at={c.line1} step={0.5} dur={1.6} style={{ ...headline, fontSize: 100, textAlign: 'center' }} />
        <Words text="to drink from Mímir’s well." at={c.line2} step={0.42} dur={1.6} style={{ ...headline, fontSize: 100, textAlign: 'center', marginTop: 8 }} />
      </AbsoluteFill>
      <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
        <Words text="Yours is *free.*" at={c.free} step={0.42} dur={1.6} style={{ ...headline, fontSize: 168 }} />
      </AbsoluteFill>
    </AbsoluteFill>
  )
}

export function Lockup({ id, word, tag, draw, fill, rune, size = 250 }: { id: string; word: number; tag: number; draw: number; fill: number; rune: number[]; size?: number }) {
  const t = useBeat()
  const w = p(t, word, 1.4)
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <Mark id={id} size={size} draw={draw} fill={fill} rune={rune} />
      <Grow k={p(t, word, 1.6, ease.io)}>
        <div style={{ ...headline, fontSize: 168, letterSpacing: '0.01em', paddingTop: 34, opacity: w, filter: w < 1 ? `blur(${(1 - w) * 12}px)` : undefined, translate: `0 ${(1 - w) * 30}px`, textAlign: 'center' }}>Mimir</div>
      </Grow>
      <Grow k={p(t, tag, 1.2, ease.io)}>
        <Words text="Your own AI assistant, always on." at={tag} step={0.22} style={{ ...gloss, fontSize: 52, paddingTop: 18, textAlign: 'center' }} />
      </Grow>
    </div>
  )
}

export function Title() {
  const t = useBeat()
  const c = cues('title')
  const rune = [0, 1, 2, 3].map((i) => p(t, c.rune + i * 0.3, 0.8, ease.io))
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      <Ripple x={960} y={540} at={c.word} size={1300} rings={3} strength={0.22} beats={5} />
      <Lockup id="t" word={c.word} tag={c.tag} draw={p(t, c.draw, 2.2, ease.io)} fill={p(t, c.draw + 1, 1.8, ease.soft)} rune={rune} />
    </AbsoluteFill>
  )
}

export function Outro() {
  const t = useBeat()
  const c = cues('outro')
  const hit = p(t, c.hit, 2.2, ease.out)
  const url = p(t, c.url, 1.4)
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      <AbsoluteFill style={{ background: 'radial-gradient(circle at 50% 46%, rgba(223,193,154,0.22), transparent 55%)', opacity: 1 - p(t, c.hit, 2.5, ease.out) }} />
      <Ripple x={960} y={470} at={c.hit} size={2200} rings={5} strength={0.4} beats={8} />
      <div style={{ scale: 1.08 - 0.08 * hit, opacity: p(t, c.hit, 0.2, ease.lin), display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
        <Lockup id="o" word={c.hit - 2} tag={c.tag} draw={1} fill={1} rune={[1, 1, 1, 1]} size={210} />
        <div style={{ marginTop: 54, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14, opacity: url, translate: `0 ${(1 - url) * 20}px` }}>
          <div style={{ fontFamily: F.sans, fontSize: 40, fontWeight: 500, letterSpacing: '0.01em', color: C.foam }}>heymimir.app</div>
          <div style={{ fontFamily: F.mono, fontSize: 21, letterSpacing: '0.18em', textTransform: 'uppercase', color: C.ink3 }}>Free · open source · macOS · Windows · Linux</div>
        </div>
      </div>
      <AbsoluteFill style={{ background: '#000', opacity: p(t, c.fade, 3, ease.io) }} />
    </AbsoluteFill>
  )
}
