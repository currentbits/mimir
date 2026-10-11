// The guide's pieces: step headers and the six-step rail, the app's window, buttons, fields and chat, a phone,
// and a pointer that travels through waypoints. Everything else comes from ../lib (the film's clock and look).
import React, { type CSSProperties, type ReactNode } from 'react'
import { AbsoluteFill } from 'remotion'
import G from '../guide.json'
import { BOT, C, Cursor, ease, eyebrow, F, Face, gloss, Grow, headline, Icon, lerp, p, Panel, pigment, Rise, Stream, Typed, useBeat, Words } from '../lib'

export const ADA = pigment(BOT.ada)
export const OK = '#2B8E59' // live, on paper

// ---------------- headers ----------------

/** A setup step: its number, what to do, and a line on why or how. Left column. */
export function StepSide({ n, title, at = 0, gl, glAt, width = 600, size = 92 }: { n: number; title: string; at?: number; gl?: string; glAt?: number; width?: number; size?: number }) {
  const t = useBeat()
  const k = p(t, at, 1.2)
  return (
    <div style={{ width, flex: 'none' }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 26, opacity: k, translate: `0 ${(1 - k) * 30}px`, filter: k < 1 ? `blur(${(1 - k) * 8}px)` : undefined }}>
        <span style={{ fontFamily: F.serif, fontSize: 200, lineHeight: 0.8, color: C.gall, fontFeatureSettings: '"lnum"' }}>{n}</span>
        <span style={{ ...eyebrow, paddingBottom: 10 }}>Step {n} of 6</span>
      </div>
      <Words text={title} at={at + 0.3} step={0.18} style={{ ...headline, fontSize: size, marginTop: 34 }} />
      {gl && <Rise at={glAt ?? at + 3} style={{ marginTop: 34 }}><p style={{ ...gloss, fontSize: 36, maxWidth: width - 20 }}>{gl}</p></Rise>}
    </div>
  )
}

/** Part two's header: what to try, across the top. */
export function Head({ title, at = 0, top = 96, size = 92, align = 'center' }: { title: string; at?: number; top?: number; size?: number; align?: 'center' | 'left' }) {
  return (
    <div style={{ position: 'absolute', top, left: align === 'left' ? 150 : 0, right: align === 'left' ? undefined : 0, textAlign: align }}>
      <Words text={title} at={at} step={0.18} style={{ ...headline, fontSize: size }} />
    </div>
  )
}

/** The line at the bottom that says why it matters. */
export function Gloss({ at, children, bottom = 70, left }: { at: number; children: ReactNode; bottom?: number; left?: number }) {
  return (
    <div style={{ position: 'absolute', bottom, left: left ?? 0, right: left === undefined ? 0 : undefined, textAlign: left === undefined ? 'center' : 'left' }}>
      <Rise at={at}><p style={{ ...gloss, fontSize: 38 }}>{children}</p></Rise>
    </div>
  )
}

/** A chapter card: "Part one / Set it up." */
export function Card({ kicker, title, gl, glAt, at = 0 }: { kicker: string; title: string; gl: string; glAt: number; at?: number }) {
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center', textAlign: 'center' }}>
      <Rise at={at} y={20}><div style={{ ...eyebrow, fontSize: 26 }}>{kicker}</div></Rise>
      <Words text={title} at={at + 0.2} step={0.22} style={{ ...headline, fontSize: 168, marginTop: 30 }} />
      <Rise at={glAt}><p style={{ ...gloss, fontSize: 46, marginTop: 34 }}>{gl}</p></Rise>
    </AbsoluteFill>
  )
}

// ---------------- the six steps, across the top while you set up ----------------

const STEPS = ['Download', 'Your AI', 'First bot', 'Your phone', 'Email & calendar', 'Skills']
const S = G.scenes
const STARTS = [S.get.at, S.account.at, S.name.at, S.pocket.at, S.plug.at, S.library.at].map((b) => b * 4)
const ALL_DONE = S.library.at * 4 + (S.library.cues.done[0] as number)
const RAIL_IN = S.setup.at * 4 + (S.setup.cues.rail[0] as number)
const RAIL_OUT = S.daily.at * 4

/** Where you are in setup. Lives outside the scenes (in Guide), so it stays put while they cut. */
export function Rail() {
  const t = useBeat() // beats since the guide began
  if (t < RAIL_IN - 0.1 || t > RAIL_OUT + 1) return null
  const out = p(t, RAIL_OUT - 0.25, 0.8, ease.in)
  const cur = STARTS.filter((s) => t >= s).length - 1
  return (
    <AbsoluteFill style={{ pointerEvents: 'none' }}>
      <div style={{ position: 'absolute', top: 40, left: 0, right: 0, display: 'flex', justifyContent: 'center', alignItems: 'center', gap: 0, opacity: 1 - out, translate: `0 ${-out * 30}px`, filter: out ? `blur(${out * 8}px)` : undefined }}>
        {STEPS.map((label, i) => {
          const k = p(t, RAIL_IN + i * 0.18, 0.8, ease.out)
          const done = i < cur || t >= ALL_DONE
          const on = i === cur && !done
          const lit = p(t, STARTS[i], 0.6)
          return (
            <React.Fragment key={label}>
              {i > 0 && <span style={{ width: 54, height: 1.5, margin: '0 16px', background: C.foam, opacity: (done ? 0.4 : 0.14) * k }} />}
              <span style={{ display: 'flex', alignItems: 'center', gap: 12, opacity: k * (on ? 1 : done ? 0.75 : 0.42), translate: `0 ${(1 - k) * 14}px` }}>
                <span style={{ width: 36, height: 36, borderRadius: 18, display: 'grid', placeItems: 'center', fontFamily: F.serif, fontSize: 24, lineHeight: 1, fontFeatureSettings: '"lnum"',
                  background: on ? `rgba(223,193,154,${0.9 * lit})` : 'transparent', color: on ? C.deep : C.gall, boxShadow: on ? undefined : `inset 0 0 0 1.5px rgba(223,193,154,0.55)` }}>
                  {done ? <Icon name="check" size={20} /> : i + 1}
                </span>
                <span style={{ fontFamily: F.mono, fontSize: 18, letterSpacing: '0.14em', textTransform: 'uppercase', color: on ? C.foam : C.ink2, whiteSpace: 'nowrap' }}>{label}</span>
              </span>
            </React.Fragment>
          )
        })}
      </div>
    </AbsoluteFill>
  )
}

// ---------------- the app ----------------

/** A block placed at x, y (w × h) that rises into place at `at`, tilted back as it comes. */
export function Enter({ at, x, y, w, h, children, from = 70, style }: { at: number; x: number; y: number; w: number; h?: number; children: ReactNode; from?: number; style?: CSSProperties }) {
  const t = useBeat()
  const k = p(t, at, 1.3)
  if (k <= 0) return null
  return (
    <div style={{ position: 'absolute', left: x, top: y, width: w, height: h, opacity: Math.min(1, k * 1.4), transform: `perspective(2400px) translateY(${(1 - k) * from}px) rotateX(${(1 - k) * 10}deg)`, filter: k < 1 ? `blur(${(1 - k) * 6}px)` : undefined, ...style }}>
      {children}
    </div>
  )
}

/** A window of the app (dark), with the traffic lights and an optional title. */
export function Win({ title, children, style, bar = true }: { title?: ReactNode; children: ReactNode; style?: CSSProperties; bar?: boolean }) {
  return (
    <Panel style={{ display: 'flex', flexDirection: 'column', ...style }}>
      {bar && (
        <div style={{ height: 46, flex: 'none', display: 'flex', alignItems: 'center', gap: 9, padding: '0 20px', background: '#1E1E21', borderBottom: `1px solid ${C.line}`, position: 'relative' }}>
          {['#FF5F57', '#FEBC2E', '#28C840'].map((x) => <span key={x} style={{ width: 14, height: 14, borderRadius: 7, background: x, opacity: 0.85 }} />)}
          {title && <span style={{ position: 'absolute', left: 0, right: 0, textAlign: 'center', fontSize: 19, color: C.text2, fontWeight: 600 }}>{title}</span>}
        </div>
      )}
      <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>{children}</div>
    </Panel>
  )
}

type Kind = 'primary' | 'plain' | 'ghost' | 'ok'
/** A button. `press` (0/1) squeezes it; `hot` lights it as the pointer arrives. */
export function Btn({ kind = 'plain', icon, press = 0, hot = 0, children, style, size = 23 }: { kind?: Kind; icon?: string; press?: number; hot?: number; children: ReactNode; style?: CSSProperties; size?: number }) {
  const bg = { primary: C.text, plain: `rgba(255,255,255,${0.08 + 0.06 * hot})`, ghost: `rgba(255,255,255,${0.06 * hot})`, ok: 'rgba(76,191,130,0.16)' }[kind]
  const fg = { primary: C.app, plain: C.text, ghost: C.text2, ok: C.live }[kind]
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 10, padding: '11px 20px', borderRadius: 11, background: bg, color: fg, fontSize: size, fontWeight: 550, whiteSpace: 'nowrap', scale: 1 - press * 0.06, boxShadow: kind === 'plain' ? `inset 0 0 0 1px ${C.line}` : undefined, filter: kind === 'primary' && hot ? `brightness(${1 - 0.08 * hot})` : undefined, ...style }}>
      {icon && <Icon name={icon} size={size} />}{children}
    </span>
  )
}

/** A status pill: green with a tick when it's working, amber while it waits. */
export function Badge({ live, children, k = 1 }: { live?: boolean; children: ReactNode; k?: number }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7, padding: '5px 13px', borderRadius: 99, fontSize: 19, fontWeight: 550, whiteSpace: 'nowrap',
      color: live ? C.live : C.pending, background: live ? 'rgba(76,191,130,0.12)' : 'rgba(226,166,71,0.12)', scale: 0.85 + 0.15 * k, opacity: k }}>
      {live && <Icon name="check" size={17} />}{children}
    </span>
  )
}

/** A text field. `children` is what's in it; with `ph` and nothing in it, the placeholder shows. */
export function Field({ children, ph, focus, style, mono }: { children?: ReactNode; ph?: string; focus?: boolean; style?: CSSProperties; mono?: boolean }) {
  return (
    <div style={{ padding: '13px 18px', borderRadius: 12, background: '#1C1C1F', fontSize: 24, minHeight: 56, display: 'flex', alignItems: 'center', fontFamily: mono ? F.mono : F.sans, whiteSpace: 'nowrap', overflow: 'hidden',
      boxShadow: `inset 0 0 0 ${focus ? 2 : 1}px ${focus ? 'rgba(236,233,225,0.4)' : C.line}`, color: children ? C.text : C.text3, ...style }}>
      {children || ph}
    </div>
  )
}

/** You, in the app's chat: a bubble on the right that opens in at `at`. */
export function You({ at, children, size = 27 }: { at: number; children: ReactNode; size?: number }) {
  const t = useBeat()
  return (
    <Grow k={p(t, at, 0.8)}>
      <div style={{ display: 'flex', justifyContent: 'flex-end', paddingBottom: 24 }}>
        <div style={{ maxWidth: 720, background: C.sunken, borderRadius: 24, padding: '15px 24px', fontSize: size, lineHeight: 1.42 }}>{children}</div>
      </div>
    </Grow>
  )
}

/** A bot in the app's chat: face, name, and a reply streamed in from `at` (or `children`, shown as is). It opens at `from`. */
export function Bot({ id = BOT.ada, name = 'Ada', at, from = at - 1, text, dur = 2, wait = 'Thinking', moods, children, below, size = 27 }: {
  id?: number; name?: string; at: number; from?: number; text?: string; dur?: number; wait?: string; moods?: Parameters<typeof Face>[0]['moods']; children?: ReactNode; below?: ReactNode; size?: number
}) {
  const t = useBeat()
  return (
    <Grow k={p(t, from, 0.8)}>
      <div style={{ display: 'flex', gap: 18, paddingBottom: 18 }}>
        <Face id={id} size={44} moods={moods ?? [[0, 'idle'], [at - 1, 'thinking'], [at, 'writing'], [at + dur + 0.3, 'idle']]} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
            <span style={{ fontSize: 23, fontWeight: 600, color: pigment(id) }}>{name}</span>
            <span style={{ fontSize: 19, color: C.text3 }}>just now</span>
          </div>
          <div style={{ fontSize: size, lineHeight: 1.45, marginTop: 4 }}>
            {children ?? (t < at ? <span style={{ color: C.text3, fontStyle: 'italic' }}>{wait}…</span> : <Stream text={text ?? ''} at={at} dur={dur} />)}
          </div>
          {below}
        </div>
      </div>
    </Grow>
  )
}

/** The message box: what you type appears from `at` over `dur` beats, and it empties when you send. */
export function Composer({ text, at, dur, send, id = BOT.ada, name = 'Ada', size = 26 }: { text: string; at: number; dur: number; send: number; id?: number; name?: string; size?: number }) {
  const t = useBeat()
  const typing = t >= at && t < send
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '12px 14px 12px 16px', borderRadius: 24, background: '#1F1F22', boxShadow: `inset 0 0 0 1px ${C.line}` }}>
      <Face id={id} size={40} moods={[[0, 'idle'], [at - 0.5, 'listening'], [send, 'idle']]} />
      <div style={{ flex: 1, minWidth: 0, fontSize: size, lineHeight: 1.35, color: typing ? C.text : C.text3, whiteSpace: 'nowrap', overflow: 'hidden' }}>
        {typing ? <Typed text={text} at={at} dur={dur} /> : `Message ${name}`}
      </div>
      <div style={{ width: 46, height: 46, flex: 'none', borderRadius: 23, display: 'grid', placeItems: 'center', background: typing && t > at + 0.2 ? C.text : 'rgba(255,255,255,0.1)', color: C.app, scale: t > send - 0.15 && t < send + 0.1 ? 0.9 : 1 }}>
        <Icon name="send" size={24} />
      </div>
    </div>
  )
}

/** A quiet chip under a reply. */
export function Tag({ icon, children, k = 1, style }: { icon?: string; children: ReactNode; k?: number; style?: CSSProperties }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 9, padding: '7px 15px 7px 12px', borderRadius: 99, background: 'rgba(255,255,255,0.06)', boxShadow: `inset 0 0 0 1px ${C.line}`, color: C.text2, fontSize: 21, whiteSpace: 'nowrap', opacity: k, scale: 0.8 + 0.2 * ease.back(Math.min(1, k)), ...style }}>
      {icon && <Icon name={icon} size={20} />}{children}
    </span>
  )
}

// ---------------- the phone ----------------

/** A phone with a Telegram chat with a bot. */
export function Phone({ children, k = 1, name = 'Ada', id = BOT.ada, moods, style, clock = '9:41' }: { children: ReactNode; k?: number; name?: string; id?: number; moods?: Parameters<typeof Face>[0]['moods']; style?: CSSProperties; clock?: string }) {
  return (
    <div style={{ width: 420, height: 860, borderRadius: 68, padding: 14, background: '#0B0B0C', boxShadow: '0 40px 80px -24px rgba(0,0,0,0.9), inset 0 0 0 2px #2A2A2E', opacity: Math.min(1, k * 1.4), scale: 0.88 + 0.12 * k, translate: `0 ${(1 - k) * 60}px`, ...style }}>
      <div style={{ height: '100%', borderRadius: 54, background: '#141416', overflow: 'hidden', display: 'flex', flexDirection: 'column', fontFamily: F.sans, color: C.text }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', padding: '18px 34px 6px', fontSize: 19, fontWeight: 600 }}><span>{clock}</span><span style={{ letterSpacing: 2 }}>●●●</span></div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '12px 22px 16px', borderBottom: `1px solid ${C.line}` }}>
          <span style={{ rotate: '180deg', display: 'flex' }}><Icon name="chevron" size={26} /></span>
          <Face id={id} size={48} moods={moods} />
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 23, fontWeight: 600 }}>{name}</div>
            <div style={{ fontSize: 17, color: C.text3, display: 'flex', alignItems: 'center', gap: 6 }}><Icon name="telegram" size={16} />Telegram · bot</div>
          </div>
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', padding: '0 16px 16px' }}>{children}</div>
        <div style={{ margin: '0 16px 22px', padding: '13px 20px', borderRadius: 24, background: '#202024', color: C.text3, fontSize: 20 }}>Message</div>
      </div>
    </div>
  )
}

/** A message on the phone, opening in at `at`. */
export function Bubble({ me, at, children, id = BOT.ada }: { me?: boolean; at: number; children: ReactNode; id?: number }) {
  const t = useBeat()
  const k = p(t, at, 0.7)
  return (
    <Grow k={k}>
      <div style={{ display: 'flex', justifyContent: me ? 'flex-end' : 'flex-start', gap: 10, alignItems: 'flex-end', paddingTop: 12, opacity: k, scale: 0.92 + 0.08 * k, transformOrigin: me ? '100% 100%' : '0 100%' }}>
        {!me && <Face id={id} size={36} />}
        <div style={{ maxWidth: 300, padding: '12px 17px', borderRadius: 20, fontSize: 24, lineHeight: 1.35, background: me ? '#3A3A40' : '#232327', color: C.text, borderBottomLeftRadius: me ? 20 : 6, borderBottomRightRadius: me ? 6 : 20 }}>{children}</div>
      </div>
    </Grow>
  )
}

// ---------------- the pointer ----------------

type Way = [beat: number, x: number, y: number]
/** Where a pointer is at beat t: it arrives at each waypoint on its beat, gliding from the one before with a small arc. */
export function track(t: number, ways: Way[]) {
  if (t <= ways[0][0]) return { x: ways[0][1], y: ways[0][2] }
  for (let i = 1; i < ways.length; i++) {
    const [b, x, y] = ways[i]
    if (t < b) {
      const [a, x0, y0] = ways[i - 1]
      const k = p(t, a, b - a, ease.io)
      return { x: lerp(x0, x, k), y: lerp(y0, y, k) - Math.sin(k * Math.PI) * Math.min(40, Math.hypot(x - x0, y - y0) * 0.1) }
    }
  }
  const [, x, y] = ways[ways.length - 1]
  return { x, y }
}
/** 1 while the pointer is near beat `at` (lights a button as it arrives). */
export const near = (t: number, at: number, before = 0.6) => (t > at - before && t < at + 0.3 ? 1 : 0)

/** The pointer on its waypoints, pressing on `clicks`, from `from` until `to`. */
export function Pointer({ ways, clicks, from, to = 999 }: { ways: Way[]; clicks: number[]; from?: number; to?: number }) {
  const t = useBeat()
  const start = from ?? ways[0][0] - 0.4
  if (t < start || t > to) return null
  const { x, y } = track(t, ways)
  const pr = clicks.some((c) => t >= c - 0.12 && t < c + 0.18) ? 1 : 0
  const k = Math.min(p(t, start, 0.4), 1 - p(t, to - 0.4, 0.4))
  return <div style={{ opacity: k }}><Cursor x={x} y={y} press={pr} /></div>
}
