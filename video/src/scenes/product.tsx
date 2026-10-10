// What it is: an app that runs AI assistants for you; you message them wherever you already talk; they remember you.
import type { CSSProperties, ReactNode } from 'react'
import { AbsoluteFill, interpolate } from 'remotion'
import { BOT, C, Chip, Cursor, cues, ease, F, Face, gloss, Grow, headline, Icon, lerp, p, Paper, Panel, pigment, Ripple, Rise, Stream, Typed, useBeat, Words } from '../lib'
import { Mark } from './open'

const ADA = pigment(BOT.ada)

// ---------------- Chat: one message, a routine and a memory ----------------

const ASK = 'Every Friday at 5pm, remind me to submit my timesheet. And keep answers short.'
const REPLY = 'Done. Your **timesheet** reminder is set for every Friday at 5 pm, and I noted that you prefer short answers.'

function Who({ name, when, size = 24 }: { name: string; when: string; size?: number }) {
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
      <span style={{ fontSize: size, fontWeight: 600, color: ADA }}>{name}</span>
      <span style={{ fontSize: size - 4, color: C.text3 }}>{when}</span>
    </div>
  )
}

export function Chat() {
  const t = useBeat()
  const c = cues('chat')
  const enter = p(t, c.card, 1.8, ease.out)
  const out = p(t, c.out, 0.75, ease.in)
  const sent = t >= c.send
  const step = t < c.plan ? 'Searching memory' : 'Scheduling'
  const pop = (at: number) => p(t, at, 0.7, ease.back)
  return (
    <AbsoluteFill style={{ padding: '84px 150px 80px' }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', justifyContent: 'space-between', gap: 60, opacity: 1 - out }}>
        <Words text="Message it like a person." at={c.head} step={0.2} style={{ ...headline, fontSize: 88, whiteSpace: 'nowrap' }} />
        <Rise at={c.gloss}><p style={{ ...gloss, fontSize: 34, maxWidth: 540, textAlign: 'right', paddingBottom: 6 }}>One message: a reminder set, and a preference remembered.</p></Rise>
      </div>
      <div style={{ flex: 1, minHeight: 0, marginTop: 40, opacity: enter * (1 - out),
        transform: `perspective(2400px) translate(${-out * 265}px, ${(1 - enter) * 180}px) rotateX(${(1 - enter) * 14}deg) scale(${1 - out * 0.55})` }}>
        <Panel style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
          <div style={{ height: 46, flex: 'none', display: 'flex', alignItems: 'center', gap: 9, padding: '0 20px', background: '#1E1E21', borderBottom: `1px solid ${C.line}` }}>
            {['#FF5F57', '#FEBC2E', '#28C840'].map((x) => <span key={x} style={{ width: 14, height: 14, borderRadius: 7, background: x, opacity: 0.85 }} />)}
          </div>
          <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
            <Sidebar />
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 18, padding: '18px 34px', borderBottom: `1px solid ${C.line}` }}>
                <Face id={BOT.ada} size={52} />
                <div>
                  <div style={{ fontFamily: F.ui, fontSize: 29, fontWeight: 500 }}>Ada</div>
                  <div style={{ fontSize: 19, color: C.text3, marginTop: 2 }}>Built-in chat · 6 memories, shared by all your bots</div>
                </div>
              </div>
              <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', padding: '0 40px 10px', overflow: 'hidden' }}>
                <div style={{ opacity: 0.42, display: 'flex', justifyContent: 'flex-end', paddingBottom: 26 }}>
                  <div style={{ maxWidth: 680, background: C.sunken, borderRadius: 24, padding: '16px 24px', fontSize: 27, lineHeight: 1.42 }}>Remember that my dentist is Dr. Naledi Lee in Stellenbosch.</div>
                </div>
                <div style={{ opacity: 0.42, display: 'flex', gap: 18, paddingBottom: 30 }}>
                  <Face id={BOT.ada} size={44} />
                  <div>
                    <Who name="Ada" when="3 h ago" />
                    <div style={{ fontSize: 27, lineHeight: 1.45, marginTop: 4 }}>Saved: Dr. Naledi Lee in Stellenbosch. Want me to remind you before your next check-up?</div>
                  </div>
                </div>
                <Grow k={p(t, c.send, 0.8, ease.out)}>
                  <div style={{ display: 'flex', justifyContent: 'flex-end', paddingBottom: 30 }}>
                    <div style={{ maxWidth: 680, background: C.sunken, borderRadius: 24, padding: '18px 26px', fontSize: 28, lineHeight: 1.42 }}>{ASK}</div>
                  </div>
                </Grow>
                <Grow k={p(t, c.search - 0.3, 0.8, ease.out)}>
                  <div style={{ display: 'flex', gap: 18, paddingBottom: 8 }}>
                    <Face id={BOT.ada} size={44} moods={[[0, 'idle'], [c.search - 0.3, 'searching'], [c.plan, 'thinking'], [c.reply, 'writing'], [c.chip1, 'happy']]} />
                    <div style={{ flex: 1 }}>
                      <Who name="Ada" when="just now" />
                      <div style={{ fontSize: 29, lineHeight: 1.45, marginTop: 4, minHeight: 84 }}>
                        {t < c.reply
                          ? <span style={{ color: C.text3, fontStyle: 'italic' }}>{step}…</span>
                          : <Stream text={REPLY} at={c.reply} dur={2.6} />}
                      </div>
                      <Grow k={p(t, c.chip1, 0.6, ease.out)}>
                        <div style={{ display: 'flex', gap: 12, paddingTop: 16 }}>
                          <Chip icon="clock" style={{ scale: pop(c.chip1) }}>Scheduled · Every Friday at 5:00 PM</Chip>
                          <Chip icon="memory" style={{ scale: pop(c.chip2) }}>Saved to memory · Answer style</Chip>
                        </div>
                      </Grow>
                    </div>
                  </div>
                </Grow>
              </div>
              <div style={{ padding: '14px 30px 26px' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '13px 16px 13px 18px', borderRadius: 24, background: '#1F1F22', boxShadow: `inset 0 0 0 1px ${C.line}` }}>
                  <Face id={BOT.ada} size={42} moods={[[0, 'idle'], [c.type - 0.5, 'listening'], [c.send, 'idle']]} />
                  <div style={{ flex: 1, fontSize: 27, lineHeight: 1.35, color: sent || t < c.type ? C.text3 : C.text, minHeight: 36 }}>
                    {sent || t < c.type ? 'Message Ada' : <Typed text={ASK} at={c.type} dur={2} />}
                  </div>
                  <div style={{ width: 48, height: 48, borderRadius: 24, display: 'grid', placeItems: 'center', background: !sent && t > c.type + 0.2 ? C.text : 'rgba(255,255,255,0.1)', color: C.app, scale: t > c.send - 0.15 && t < c.send + 0.1 ? 0.9 : 1 }}>
                    <Icon name="send" size={25} />
                  </div>
                </div>
              </div>
            </div>
          </div>
        </Panel>
      </div>
    </AbsoluteFill>
  )
}

/** The app's sidebar, as in the real thing: what needs you, your bots, your chats. */
function Sidebar() {
  const label: CSSProperties = { fontSize: 18, color: C.text3, padding: '22px 14px 8px', fontWeight: 500 }
  const row = (icon: ReactNode, name: string, sub: string, on?: boolean) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 14px', borderRadius: 12, background: on ? 'rgba(255,255,255,0.08)' : undefined }}>
      {icon}
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 22, whiteSpace: 'nowrap' }}>{name}</div>
        <div style={{ fontSize: 17, color: C.text3, marginTop: 1, whiteSpace: 'nowrap' }}>{sub}</div>
      </div>
    </div>
  )
  const box = (name: string) => <span style={{ width: 42, height: 42, flex: 'none', borderRadius: 11, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,0.06)', color: C.text2 }}><Icon name={name} size={22} /></span>
  return (
    <div style={{ width: 330, flex: 'none', background: '#1D1D20', borderRight: `1px solid ${C.line}`, padding: '16px 14px', fontFamily: F.sans, color: C.text }}>
      {row(box('inbox'), 'Needs you', 'All clear')}
      <div style={label}>Bots</div>
      {row(<Face id={BOT.ada} size={42} />, 'Ada', 'Telegram')}
      {row(<Face id={BOT.researcher} size={42} />, 'Researcher', 'On the web')}
      {row(<Face id={BOT.briefer} size={42} />, 'Morning briefer', '7:30 daily')}
      <div style={label}>Chats</div>
      {row(box('laptop'), 'This computer', 'Built-in chat', true)}
      {row(box('telegram'), 'Telegram', 'With Ada')}
      {row(box('slack'), 'Slack DM', 'With Researcher')}
    </div>
  )
}

// ---------------- How: what Mimir is, as a picture ----------------

const YOU: [string, string][] = [['telegram', 'Telegram'], ['slack', 'Slack'], ['discord', 'Discord'], ['laptop', 'The Mimir app']]
const AI = ['Claude', 'ChatGPT', 'Grok', 'Any API key']
const TOOLS: [string, string][] = [['inbox', 'Email'], ['clock', 'Calendar'], ['globe', 'The web'], ['browser', 'A browser']]
const ROWS = [430, 505, 580, 655]
const HUB = { x: 960, y: 540, s: 190 }

function Node({ k, children, style }: { k: number; children: ReactNode; style?: CSSProperties }) {
  return <div style={{ position: 'absolute', opacity: Math.min(1, k * 1.5), scale: 0.85 + 0.15 * k, filter: k < 1 ? `blur(${(1 - k) * 8}px)` : undefined, ...style }}>{children}</div>
}
function Pill({ icon, children, w = 290 }: { icon?: string; children: ReactNode; w?: number }) {
  return (
    <div style={{ width: w, height: 60, display: 'flex', alignItems: 'center', gap: 14, padding: '0 22px', borderRadius: 30, background: 'rgba(236,233,225,0.07)', boxShadow: 'inset 0 0 0 1px rgba(236,233,225,0.12)', color: C.foam, fontFamily: F.sans, fontSize: 25 }}>
      {icon && <span style={{ color: C.ink3, display: 'flex' }}><Icon name={icon} size={24} /></span>}{children}
    </div>
  )
}
const caption: CSSProperties = { ...gloss, fontSize: 32, textAlign: 'center', whiteSpace: 'nowrap' }

export function How() {
  const t = useBeat()
  const c = cues('how')
  const app = p(t, c.app, 0.9, ease.back)
  const faces = [0, 1, 2].map((i) => p(t, c.faces + i * 0.25, 0.8, ease.back))
  const you = (i: number) => p(t, c.you + i * 0.2, 0.8)
  const ai = (i: number) => p(t, c.ai + i * 0.2, 0.8)
  const tools = (i: number) => p(t, c.tools + i * 0.15, 0.8)
  const L = 250 + 290, R = 1380
  const left = ROWS.map((y) => `M${L} ${y} C ${L + 170} ${y}, ${HUB.x - HUB.s / 2 - 160} ${HUB.y}, ${HUB.x - HUB.s / 2} ${HUB.y}`)
  const right = ROWS.map((y) => `M${HUB.x + HUB.s / 2} ${HUB.y} C ${HUB.x + HUB.s / 2 + 160} ${HUB.y}, ${R - 170} ${y}, ${R} ${y}`)
  const toolX = TOOLS.map((_, i) => 960 + (i - 1.5) * 250)
  const down = toolX.map((x) => `M960 790 C 960 830, ${x} 820, ${x} 858`)
  const draw = (at: number, i: number) => p(t, at + 0.3 + i * 0.1, 1, ease.io)
  // One message goes round: from you, to Mimir, out to your AI, back, and home again.
  const hop = (a: number, b: number) => p(t, a, b - a, ease.io)
  const dot = (d: string, k: number, x0: number, y0: number, x1: number, y1: number) => {
    if (k <= 0 || k >= 1) return null
    const [, cx1, cy1, cx2, cy2] = d.match(/C ([\d.]+) ([\d.]+), ([\d.]+) ([\d.]+)/)!.map(Number)
    const b = (a0: number, a1: number, a2: number, a3: number) => (1 - k) ** 3 * a0 + 3 * (1 - k) ** 2 * k * a1 + 3 * (1 - k) * k * k * a2 + k ** 3 * a3
    return <circle cx={b(x0, cx1, cx2, x1)} cy={b(y0, cy1, cy2, y1)} r={9} fill={ADA} />
  }
  const f = c.flow
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', top: 110, left: 0, right: 0, textAlign: 'center' }}>
        <Words text="A free app that runs AI assistants *for* *you.*" at={c.head} step={0.18} style={{ ...headline, fontSize: 86 }} />
      </div>
      <svg width={1920} height={1080} style={{ position: 'absolute', inset: 0 }}>
        {left.map((d, i) => <path key={`l${i}`} d={d} fill="none" stroke={C.foam} strokeOpacity={0.22} strokeWidth={2.5} pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - draw(c.you, i)} />)}
        {right.map((d, i) => <path key={`r${i}`} d={d} fill="none" stroke={C.foam} strokeOpacity={0.22} strokeWidth={2.5} pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - draw(c.ai, i)} />)}
        {down.map((d, i) => <path key={`d${i}`} d={d} fill="none" stroke={C.foam} strokeOpacity={0.22} strokeWidth={2.5} pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - draw(c.tools, i)} />)}
        {dot(left[0], hop(f, f + 0.75), L, ROWS[0], HUB.x - HUB.s / 2, HUB.y)}
        {dot(right[1], hop(f + 0.75, f + 1.5), HUB.x + HUB.s / 2, HUB.y, R, ROWS[1])}
        {dot(right[1], 1 - hop(f + 1.5, f + 2.25), HUB.x + HUB.s / 2, HUB.y, R, ROWS[1])}
        {dot(down[0], hop(f + 2.25, f + 3), 960, 790, toolX[0], 858)}
        {dot(left[0], 1 - hop(f + 3, f + 3.75), L, ROWS[0], HUB.x - HUB.s / 2, HUB.y)}
      </svg>
      <Ripple x={HUB.x} y={HUB.y} at={f + 0.75} size={520} rings={2} strength={0.3} beats={2} />
      <Ripple x={HUB.x} y={HUB.y} at={f + 2.25} size={520} rings={2} strength={0.3} beats={2} />
      {/* You */}
      {YOU.map(([icon, name], i) => <Node key={name} k={you(i)} style={{ left: 250, top: ROWS[i] - 30 }}><Pill icon={icon}>{name}</Pill></Node>)}
      <Node k={p(t, c.you + 0.5, 1)} style={{ left: 250, width: 290, top: 712 }}><div style={caption}>you message them</div></Node>
      {/* Mimir, with its bots peeking out */}
      {[BOT.researcher, BOT.ada, BOT.briefer].map((id, i) => (
        <div key={id} style={{ position: 'absolute', left: HUB.x - 37 + (i - 1) * 76, top: HUB.y - HUB.s / 2 - 50 + Math.abs(i - 1) * 8 + (1 - faces[i]) * 60, opacity: Math.min(1, faces[i] * 2), zIndex: 0 }}>
          <Face id={id} size={74} moods={[[0, 'happy'], [c.faces + 1.5, 'idle'], [f, 'attention']]} />
        </div>
      ))}
      <div style={{ position: 'absolute', left: HUB.x - HUB.s / 2, top: HUB.y - HUB.s / 2, width: HUB.s, height: HUB.s, scale: 0.7 + 0.3 * app, opacity: Math.min(1, app * 1.5), zIndex: 1,
        borderRadius: 44, background: 'linear-gradient(#2B1F13, #0E0A06)', boxShadow: '0 30px 60px -20px rgba(0,0,0,0.9), inset 0 0 0 1.5px rgba(255,240,220,0.1)', display: 'grid', placeItems: 'center' }}>
        <Mark id="h" size={150} />
      </div>
      <Node k={p(t, c.app + 0.5, 1)} style={{ left: HUB.x - 300, width: 600, top: HUB.y + HUB.s / 2 + 22, textAlign: 'center' }}>
        <div style={{ fontFamily: F.serif, fontSize: 46, color: C.foam, lineHeight: 1 }}>Mimir</div>
        <div style={{ ...caption, marginTop: 10 }}>on your computer</div>
      </Node>
      {/* Your AI */}
      {AI.map((name, i) => <Node key={name} k={ai(i)} style={{ left: R, top: ROWS[i] - 30 }}><Pill>{name}</Pill></Node>)}
      <Node k={p(t, c.ai + 0.5, 1)} style={{ left: R - 80, width: 450, top: 712 }}><div style={caption}>on the AI plan you already pay for</div></Node>
      {/* Your things */}
      {TOOLS.map(([icon, name], i) => <Node key={name} k={tools(i)} style={{ left: toolX[i] - 115, top: 858 }}><Pill icon={icon} w={230}>{name}</Pill></Node>)}
    </AbsoluteFill>
  )
}

// ---------------- Channels: where you already talk ----------------

const CHANNELS: [string, string][] = [['telegram', 'Telegram,'], ['slack', 'Slack,'], ['discord', 'Discord,'], ['laptop', 'or your desktop.']]

function Bubble({ me, children, at }: { me?: boolean; children: ReactNode; at: number }) {
  const t = useBeat()
  const k = p(t, at, 0.7, ease.out)
  return (
    <Grow k={k}>
      <div style={{ display: 'flex', justifyContent: me ? 'flex-end' : 'flex-start', gap: 10, alignItems: 'flex-end', paddingTop: 14, opacity: k, scale: 0.92 + 0.08 * k, transformOrigin: me ? '100% 100%' : '0 100%' }}>
        {!me && <Face id={BOT.ada} size={38} />}
        <div style={{ maxWidth: 300, padding: '13px 18px', borderRadius: 20, fontSize: 25, lineHeight: 1.35, background: me ? '#3A3A40' : '#232327', color: C.text, borderBottomLeftRadius: me ? 20 : 6, borderBottomRightRadius: me ? 6 : 20 }}>{children}</div>
      </div>
    </Grow>
  )
}

export function Channels() {
  const t = useBeat()
  const c = cues('channels')
  const times = [c.tg, c.slack, c.discord, c.desk]
  const enter = p(t, c.phone, 1.4, ease.out)
  const cur = times.filter((x) => t >= x).length - 1
  return (
    <AbsoluteFill style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 150 }}>
      <div style={{ width: 430, height: 880, borderRadius: 70, padding: 14, background: '#0B0B0C', boxShadow: `0 40px 80px -24px rgba(0,0,0,0.9), inset 0 0 0 2px #2A2A2E`, opacity: enter, scale: 0.86 + 0.14 * enter, translate: `${(1 - enter) * -40}px 0` }}>
        <div style={{ height: '100%', borderRadius: 56, background: '#141416', overflow: 'hidden', display: 'flex', flexDirection: 'column', fontFamily: F.sans, color: C.text }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', padding: '18px 34px 6px', fontSize: 19, fontWeight: 600 }}><span>7:42</span><span style={{ letterSpacing: 2 }}>●●●</span></div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '12px 22px 16px', borderBottom: `1px solid ${C.line}` }}>
            <span style={{ rotate: '180deg', display: 'flex' }}><Icon name="chevron" size={26} /></span>
            <Face id={BOT.ada} size={48} moods={[[0, 'idle'], [c.msg2, 'reading'], [c.msg2 + 0.8, 'writing'], [c.msg3, 'happy']]} />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 23, fontWeight: 600 }}>Ada</div>
              <div style={{ fontSize: 17, color: C.text3, display: 'flex', alignItems: 'center', gap: 6 }}>
                <Icon name={CHANNELS[Math.max(0, cur)][0]} size={16} />{['Telegram', 'Slack', 'Discord', 'Mimir'][Math.max(0, cur)]} · bot
              </div>
            </div>
          </div>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', padding: '0 18px 18px' }}>
            <div style={{ opacity: 0.45 }}>
              <Bubble at={-9} me>Book padel for Thursday?</Bubble>
              <Bubble at={-9}>Booked: Court 2, 18:00.</Bubble>
            </div>
            <div style={{ textAlign: 'center', fontSize: 17, color: C.text3, padding: '18px 0 4px' }}>Today</div>
            <Bubble at={c.msg1}>Morning! Padel moved to 18:30. Shall I tell Sam?</Bubble>
            <Bubble at={c.msg2} me>Yes please</Bubble>
            <Bubble at={c.msg3}>Done. Sam knows.</Bubble>
          </div>
          <div style={{ margin: '0 16px 22px', padding: '13px 20px', borderRadius: 24, background: '#202024', color: C.text3, fontSize: 20 }}>Message</div>
        </div>
      </div>
      <div style={{ width: 860 }}>
        <Words text={'Talk to it where\nyou already talk:'} at={c.head} step={0.2} style={headline} />
        <div style={{ position: 'relative', height: 120, marginTop: 26 }}>
          {CHANNELS.map(([icon, word], i) => {
            const k = p(t, times[i], 0.6, ease.out)
            const o = i < 3 ? p(t, times[i + 1] - 0.05, 0.45, ease.in) : 0
            if (k <= 0 || o >= 1) return null
            return (
              <div key={i} style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', gap: 26, color: C.gall, opacity: k * (1 - o), translate: `0 ${(1 - k) * 60 - o * 60}px`, filter: `blur(${(1 - k) * 8 + o * 8}px)` }}>
                <Icon name={icon} size={72} />
                <span style={{ fontFamily: F.serif, fontStyle: 'italic', fontSize: 96 }}>{word}</span>
              </div>
            )
          })}
        </div>
      </div>
    </AbsoluteFill>
  )
}

// ---------------- Memory: it remembers you, and you can read, correct and undo it ----------------

type Slip = { title: string; body: string; meta: string[]; x: number; y: number; r: number }
const SLIPS: Slip[] = [
  { title: 'Dentist', body: 'Dr. Naledi Lee in Stellenbosch. Usually Tuesday mornings.', meta: ['you said', '12 Sep'], x: 455, y: 470, r: -2.6 },
  { title: 'Partner', body: 'Sam. Anniversary is 14 March.', meta: ['you said', '20 Sep'], x: 960, y: 440, r: 1.3 },
  { title: 'Answer style', body: 'Keep answers short.', meta: ['told in chat', 'just now'], x: 1465, y: 482, r: 2.4 },
  { title: 'Padel', body: 'Court 2 at Sea Point, Thursdays at 18:00.', meta: ['Ada, in a chat', '2 h ago'], x: 700, y: 740, r: -1.5 },
  { title: 'Invoice Acme', body: 'Send the September invoice before Friday.', meta: ['task', 'due Friday'], x: 1220, y: 748, r: 1.9 },
]
const meta: CSSProperties = { fontFamily: F.mono, fontSize: 17, color: C.pInk3, display: 'flex', gap: 18, marginTop: 16, letterSpacing: '0.02em' }

function SlipCard({ s, children }: { s: Slip; children?: ReactNode }) {
  return (
    <Paper style={{ width: 470, padding: '26px 30px 22px' }}>
      <div style={{ fontFamily: F.serif, fontSize: 40, lineHeight: 1.05 }}>{s.title}</div>
      {children ?? <div style={{ fontSize: 26, lineHeight: 1.4, color: C.pInk2, marginTop: 10 }}>{s.body}</div>}
      {!children && <div style={meta}>{s.meta.map((m) => <span key={m}>{m}</span>)}</div>}
    </Paper>
  )
}

export function Memory() {
  const t = useBeat()
  const c = cues('memory')
  const lands = [c.s1, c.s2, c.s3, c.s4, c.s5]
  const push = p(t, c.push, 1.6, ease.io)
  const h1out = p(t, c.push, 0.6, ease.in)
  const strike = p(t, c.strike, 0.6, ease.io)
  const fixed = p(t, c.fix, 0.6, ease.out)
  const sugg = p(t, c.sugg, 0.9, ease.out)
  const accepted = t >= c.accept
  const cursorK = p(t, c.sugg + 0.4, c.accept - c.sugg - 0.5, ease.io)
  const press = t >= c.accept - 0.1 && t < c.accept + 0.2 ? 1 : 0
  return (
    <AbsoluteFill>
      {lands.map((at, i) => <Ripple key={i} x={SLIPS[i].x} y={SLIPS[i].y} at={at} size={620} rings={2} strength={0.28} beats={3} />)}
      <div style={{ position: 'absolute', top: 120, left: 0, right: 0, textAlign: 'center', opacity: 1 - h1out, filter: `blur(${h1out * 10}px)` }}>
        <Words text="It remembers you." at={c.head} step={0.25} style={{ ...headline, fontSize: 108 }} />
      </div>
      <div style={{ position: 'absolute', top: 110, left: 0, right: 0, textAlign: 'center' }}>
        <Words text="Read it. Correct it. Undo it." at={c.head2} step={0.3} style={{ ...headline, fontSize: 96 }} />
      </div>
      {SLIPS.map((s, i) => {
        const k = p(t, lands[i] - 0.75, 0.75, ease.in)
        const settle = p(t, lands[i], 0.6, ease.out)
        const focus = i === 3
        const away = focus ? 0 : p(t, c.push + i * 0.06, 1.1, ease.in)
        const x = focus ? lerp(s.x, 960, push) : s.x
        const y = focus ? lerp(s.y, 480, push) : s.y
        const sc = (k < 1 ? lerp(1.7, 1, k) : 1 - 0.015 * Math.sin(settle * Math.PI)) * (focus ? lerp(1, 1.55, push) : 1 - 0.12 * away)
        if (k <= 0) return null
        return (
          <div key={i} style={{ position: 'absolute', left: x, top: y, translate: '-50% -50%', scale: sc, rotate: `${(focus ? lerp(s.r, 0, push) : s.r) + (1 - k) * 10}deg`,
            opacity: Math.min(1, k * 1.6) * (1 - away), filter: k < 1 || away > 0 ? `blur(${(1 - k) * 14 + away * 10}px)` : undefined, zIndex: focus ? 2 : 1 }}>
            {focus && t >= c.strike - 0.5 ? (
              <SlipCard s={s}>
                <div style={{ fontSize: 26, lineHeight: 1.4, color: C.pInk2, marginTop: 10 }}>
                  Court 2 at Sea Point, Thursdays at{' '}
                  <span style={{ position: 'relative', display: 'inline-block' }}>
                    <span style={{ opacity: 1 - fixed }}>18:00.</span>
                    <span style={{ position: 'absolute', left: 0, top: 0, opacity: fixed, color: C.pInk, fontWeight: 600 }}>18:30.</span>
                    <span style={{ position: 'absolute', left: -2, top: '55%', height: 2.5, width: `${strike * (1 - fixed) * 104}%`, background: C.danger }} />
                  </span>
                </div>
                <Grow k={fixed}>
                  <div style={{ fontSize: 20, color: C.pInk3, textDecoration: 'line-through', marginTop: 10 }}>Court 2 at Sea Point, Thursdays at 18:00.</div>
                </Grow>
                <div style={meta}>
                  <span>{fixed > 0.5 ? 'you said' : s.meta[0]}</span><span>{fixed > 0.5 ? 'just now' : s.meta[1]}</span>
                  {fixed > 0.5 && <span style={{ color: '#9A6409' }}>corrects #4</span>}
                </div>
              </SlipCard>
            ) : <SlipCard s={s} />}
          </div>
        )
      })}
      {sugg > 0 && (
        <div style={{ position: 'absolute', left: 960, top: 770, translate: `-50% ${(1 - sugg) * 40}px`, opacity: sugg * (1 - p(t, c.accept + 1, 0.6, ease.in)) }}>
          <Paper style={{ display: 'flex', alignItems: 'center', gap: 22, padding: '20px 22px 20px 28px', width: 980, background: C.paper2 }}>
            <Icon name="wand" size={30} />
            <div style={{ flex: 1 }}>
              <div style={{ fontFamily: F.mono, fontSize: 16, color: C.pInk3, letterSpacing: '0.12em', textTransform: 'uppercase' }}>Suggested by the tidy-up</div>
              <div style={{ fontSize: 27, marginTop: 4 }}>Remember: Sam is your padel partner.</div>
            </div>
            {accepted ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: '#2B8E59', fontSize: 25, fontWeight: 600, padding: '0 14px', opacity: p(t, c.accept, 0.3) }}><Icon name="check" size={28} />Accepted</div>
            ) : (
              <>
                <div style={{ fontSize: 24, padding: '12px 20px', color: C.pInk2 }}>Dismiss</div>
                <div style={{ fontSize: 24, padding: '12px 24px', borderRadius: 12, background: C.pInk, color: C.paper, scale: 1 - press * 0.06 }}>Accept</div>
              </>
            )}
          </Paper>
          {!accepted || t < c.accept + 0.6 ? (
            <Cursor x={interpolate(cursorK, [0, 1], [1100, 905])} y={interpolate(cursorK, [0, 1], [320, 52])} press={press} />
          ) : null}
        </div>
      )}
      <div style={{ position: 'absolute', bottom: 92, left: 0, right: 0, textAlign: 'center' }}>
        <div style={{ opacity: 1 - h1out }}><Rise at={c.gloss}><p style={gloss}>Shared by all your bots. Every memory says who wrote it.</p></Rise></div>
      </div>
      <div style={{ position: 'absolute', bottom: 70, left: 0, right: 0, textAlign: 'center' }}>
        <Rise at={c.gloss2}><p style={gloss}>A correction keeps what it replaced. Nothing changes until you accept.</p></Rise>
      </div>
    </AbsoluteFill>
  )
}
