// What it does: routines, your mail/calendar/web, asking first, skills, several bots, meetings.
import type { CSSProperties } from 'react'
import { AbsoluteFill, random } from 'remotion'
import { BOT, C, Chip, Cursor, cues, ease, eyebrow, F, Face, gloss, glide, Grow, headline, Icon, lerp, p, Panel, Paper, pigment, press, Ripple, Rise, Stream, Typed, useBeat, Words } from '../lib'

const ADA = pigment(BOT.ada)
const side: CSSProperties = { width: 600, flex: 'none' }
const btnInk: CSSProperties = { fontSize: 25, padding: '13px 28px', borderRadius: 12, background: C.pInk, color: C.paper, fontWeight: 500 }
const btnQuiet: CSSProperties = { fontSize: 25, padding: '13px 18px', color: C.pInk2 }

function Side({ kicker, title, at, gl, glAt, size = 104 }: { kicker: string; title: string; at: number; gl?: string; glAt?: number; size?: number }) {
  return (
    <div style={side}>
      <Rise at={at} y={20}><div style={{ ...eyebrow, marginBottom: 26 }}>{kicker}</div></Rise>
      <Words text={title} at={at + 0.15} step={0.2} style={{ ...headline, fontSize: size }} />
      {gl && <Rise at={glAt ?? at + 2} style={{ marginTop: 36 }}><p style={{ ...gloss, maxWidth: 560 }}>{gl}</p></Rise>}
    </div>
  )
}

// ---------------- Routines ----------------

export function Routines() {
  const t = useBeat()
  const c = cues('routines')
  const up = p(t, c.card - 0.4, 1.3, ease.io)
  const card = p(t, c.card, 1.1, ease.out)
  const spin = p(t, c.spin, c.notify - c.spin, ease.io)
  const hours = lerp(22, 31.5, spin)
  const note = p(t, c.notify, 0.9, ease.out)
  const ran = t >= c.notify
  const quote = '“Every weekday at 7:30, brief me on tech news.”'
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', left: 0, right: 0, top: lerp(430, 230, up), display: 'flex', justifyContent: 'center', scale: lerp(1, 0.78, up) }}>
        <div style={{ ...headline, fontStyle: 'italic', fontSize: 88, maxWidth: 1500, textAlign: 'center' }}><Ghost text={quote} at={c.type} dur={2} /></div>
      </div>
      <div style={{ position: 'absolute', left: 960, top: 600, translate: `-50% ${(1 - card) * 60 - 50}%`, opacity: card, scale: 0.94 + 0.06 * card }}>
        <Paper style={{ display: 'flex', alignItems: 'center', gap: 40, padding: '34px 44px', width: 1060 }}>
          <svg width={150} height={150} viewBox="-75 -75 150 150" style={{ flex: 'none' }}>
            <circle r={68} fill="none" stroke={C.pInk} strokeOpacity={0.18} strokeWidth={3} />
            {Array.from({ length: 12 }, (_, i) => <line key={i} x1={0} y1={-60} x2={0} y2={i % 3 ? -55 : -50} stroke={C.pInk} strokeOpacity={0.5} strokeWidth={3} transform={`rotate(${i * 30})`} />)}
            <line x1={0} y1={8} x2={0} y2={-34} stroke={C.pInk} strokeWidth={6} strokeLinecap="round" transform={`rotate(${((hours % 12) / 12) * 360})`} />
            <line x1={0} y1={10} x2={0} y2={-52} stroke={ran ? pigment(BOT.briefer) : C.pInk} strokeWidth={3.5} strokeLinecap="round" transform={`rotate(${(hours % 1) * 360})`} />
            <circle r={6} fill={C.pInk} />
          </svg>
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: F.serif, fontSize: 58, lineHeight: 1.05 }}>Weekdays at 7:30 AM</div>
            <div style={{ fontSize: 30, color: C.pInk2, marginTop: 10 }}>Brief me on tech news</div>
            <div style={{ fontFamily: F.mono, fontSize: 19, color: C.pInk3, marginTop: 16, letterSpacing: '0.02em' }}>{ran ? 'Ran today 7:30 · next: tomorrow 7:30' : 'Next: tomorrow 7:30'}</div>
          </div>
          <Face id={BOT.briefer} size={110} moods={[[0, 'asleep'], [c.notify - 0.4, 'happy'], [c.notify + 1.4, 'idle']]} />
        </Paper>
      </div>
      <div style={{ position: 'absolute', right: 60, top: 48, opacity: note, translate: `${(1 - note) * 80}px 0` }}>
        <Panel style={{ width: 640, borderRadius: 26, padding: '22px 26px', display: 'flex', gap: 20, alignItems: 'center', background: 'rgba(44,44,49,0.96)' }}>
          <Face id={BOT.briefer} size={66} moods={[[0, 'writing']]} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 23 }}><b style={{ fontWeight: 600 }}>Morning briefer</b><span style={{ color: C.text3 }}>7:30</span></div>
            <div style={{ fontSize: 22, color: C.text2, marginTop: 4, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>Three things in tech today. Rust 1.96 is out, and…</div>
          </div>
        </Panel>
      </div>
      <div style={{ position: 'absolute', bottom: 96, left: 0, right: 0, textAlign: 'center' }}>
        <Rise at={c.gloss}><p style={gloss}>Quiet checks speak up only when something changes.</p></Rise>
      </div>
    </AbsoluteFill>
  )
}

/** Typed, with the rest of the line held invisibly so centred text doesn't shift. */
function Ghost({ text, at, dur }: { text: string; at: number; dur: number }) {
  const t = useBeat()
  return (
    <span style={{ position: 'relative', display: 'inline-block' }}>
      <span style={{ opacity: 0 }}>{text}</span>
      <span style={{ position: 'absolute', inset: 0, textAlign: 'left' }}><Typed text={text} at={at} dur={dur} caret={t < at + dur + 0.5} /></span>
    </span>
  )
}

// ---------------- Sources: your inbox, your calendar and the web ----------------

const ITEMS: { icon: string; head: string; body: string; at: 'mail' | 'cal' | 'web'; x: number; y: number; r: number }[] = [
  { icon: 'inbox', head: 'Acme Billing', body: 'Invoice 4411 is due Friday', at: 'mail', x: 1290, y: 470, r: -3 },
  { icon: 'clock', head: 'Tuesday 10:00', body: 'Dentist, Dr. Lee', at: 'cal', x: 1450, y: 650, r: 2.5 },
  { icon: 'globe', head: 'blog.rust-lang.org', body: 'Announcing Rust 1.96', at: 'web', x: 1270, y: 830, r: -1.5 },
]

export function Sources() {
  const t = useBeat()
  const c = cues('sources')
  const face: [number, number] = [520, 650]
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', top: 120, left: 0, right: 0, textAlign: 'center' }}>
        <Words text="Your inbox, your calendar *and* *the* *web.*" at={c.head} step={0.2} style={{ ...headline, fontSize: 96 }} />
      </div>
      <Ripple x={face[0]} y={face[1] + 90} at={c.absorb + 0.6} size={700} rings={2} strength={0.25} beats={3} />
      <div style={{ position: 'absolute', left: face[0], top: face[1], translate: '-50% -50%' }}>
        <Face id={BOT.ada} size={210} moods={[[0, 'idle'], [c.mail + 0.4, 'reading'], [c.absorb + 0.5, 'happy'], [c.reply + 0.2, 'writing'], [c.reply + 2.6, 'idle']]} />
      </div>
      {ITEMS.map((it, i) => {
        const at = c[it.at]
        const k = p(t, at, 1.1, ease.out)
        const a = p(t, c.absorb + i * 0.15, 0.8, ease.in)
        if (k <= 0 || a >= 1) return null
        const x = lerp(it.x + 900, it.x, k), y = it.y
        return (
          <div key={i} style={{ position: 'absolute', left: lerp(x, face[0], a), top: lerp(y, face[1], a) - Math.sin(a * Math.PI) * 80, translate: '-50% -50%', rotate: `${it.r + (1 - k) * 12}deg`, scale: lerp(1, 0.08, a), opacity: 1 - a * 0.6 }}>
            <Paper style={{ width: 430, padding: '22px 26px', display: 'flex', gap: 20, alignItems: 'center' }}>
              <div style={{ width: 64, height: 64, borderRadius: 16, background: 'rgba(27,28,26,0.06)', display: 'grid', placeItems: 'center', color: C.pInk2 }}><Icon name={it.icon} size={34} /></div>
              <div>
                <div style={{ fontSize: 26, fontWeight: 600 }}>{it.head}</div>
                <div style={{ fontSize: 24, color: C.pInk2, marginTop: 2 }}>{it.body}</div>
              </div>
            </Paper>
          </div>
        )
      })}
      {t >= c.reply && (
        <div style={{ position: 'absolute', left: 720, top: 470, width: 1060, opacity: p(t, c.reply, 0.6), translate: `${(1 - p(t, c.reply, 1)) * -40}px 0` }}>
          <Panel style={{ padding: '30px 36px 32px', borderTopLeftRadius: 8 }}>
            <div style={{ fontSize: 24, fontWeight: 600, color: ADA }}>Ada</div>
            <div style={{ fontSize: 33, lineHeight: 1.42, marginTop: 6 }}>
              <Stream text="Three emails need you today. Your dentist is Tuesday at 10. And Rust 1.96 is out." at={c.reply + 0.2} dur={2} />
            </div>
            <Grow k={p(t, c.reply + 2.2, 0.7)}>
              <div style={{ display: 'flex', gap: 12, paddingTop: 20 }}>
                <Chip icon="inbox">Mail · 3 of 41</Chip><Chip icon="clock">Calendar · Tue 10:00</Chip><Chip icon="globe">blog.rust-lang.org</Chip>
              </div>
            </Grow>
          </Panel>
        </div>
      )}
    </AbsoluteFill>
  )
}

// ---------------- Approve: it asks before it acts ----------------

const RULES: [string, number][] = [['Read your email', 0], ['Send email', 1], ['Delete files', 2]]
const CHOICES = ['Allow', 'Ask', 'Deny']
const TINT = ['rgba(43,142,89,0.16)', 'rgba(154,100,9,0.16)', 'rgba(191,58,50,0.14)']
const TINT_INK = ['#2B8E59', '#9A6409', '#BF3A32']

export function Approve() {
  const t = useBeat()
  const c = cues('approve')
  const card = p(t, c.card, 1.2)
  const fly = p(t, c.fly, 0.9, ease.in)
  const sent = t >= c.sent
  const cur = glide(t, c.cursor, c.click - c.cursor - 0.4, [1040, 640], [742, 410])
  const rules = p(t, c.rules, 1.1)
  const W = 860, H = 480
  return (
    <AbsoluteFill style={{ flexDirection: 'row', alignItems: 'center', padding: '0 0 0 150px', gap: 90 }}>
      <Side kicker="Approvals" title={'It asks\nbefore it acts.'} at={c.head} gl="Your rules: allow, ask or deny, tool by tool. After it reads anything from outside, changes always ask." glAt={c.gloss} />
      <div style={{ position: 'relative', width: W, height: 640 }}>
        {fly < 1 && (
          <div style={{ position: 'absolute', left: 0, top: 80, width: W, height: H, opacity: card * (1 - fly), scale: 0.95 + 0.05 * card, translate: `${fly * 700}px ${(1 - card) * 40 - fly * 120}px`, rotate: `${fly * 9}deg` }}>
            <Paper style={{ width: W, height: H, padding: '38px 40px 32px', display: 'flex', flexDirection: 'column' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 22 }}>
                <Face id={BOT.ada} size={84} moods={[[0, 'asking'], [c.sent, 'happy']]} />
                <div>
                  <div style={{ fontFamily: F.serif, fontSize: 46, lineHeight: 1.05 }}><span style={{ color: ADA }}>Ada</span> wants to send an email</div>
                  <div style={{ fontSize: 23, color: C.pInk3, marginTop: 8 }}>To sam@acme.example · “Invoice 4411”</div>
                </div>
              </div>
              <div style={{ marginTop: 28, padding: '22px 26px', borderRadius: 12, background: 'rgba(27,28,26,0.05)', fontSize: 28, lineHeight: 1.45 }}>Hi Sam, the invoice goes out on Friday. Thanks for the reminder!</div>
              <div style={{ marginTop: 'auto', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 8 }}>
                {sent ? (
                  <div style={{ ...btnInk, background: '#2B8E59', display: 'flex', alignItems: 'center', gap: 10, scale: 0.9 + 0.1 * p(t, c.sent, 0.5, ease.back) }}><Icon name="check" size={26} />Sent</div>
                ) : (
                  <>
                    <div style={btnQuiet}>Don’t send</div>
                    <div style={{ ...btnInk, scale: 1 - press(t, c.click) * 0.06, background: t > c.click - 0.6 ? '#33342F' : C.pInk }}>Send</div>
                  </>
                )}
              </div>
            </Paper>
            {t < c.fly && <Cursor x={cur.x} y={cur.y} press={press(t, c.click)} />}
          </div>
        )}
        {rules > 0 && (
          <div style={{ position: 'absolute', left: 0, top: 110, opacity: rules, translate: `0 ${(1 - rules) * 50}px` }}>
            <Paper style={{ width: W, padding: '34px 40px 18px' }}>
              <div style={{ ...eyebrow, color: C.pInk3, fontSize: 18 }}>Permissions</div>
              <div style={{ fontFamily: F.serif, fontSize: 46, marginTop: 6, marginBottom: 14 }}>What Ada may do</div>
              {RULES.map(([label, to], i) => {
                const k = p(t, [c.r1, c.r2, c.r3][i], 0.6, ease.out)
                const pos = lerp(1, to, k)
                const ink = k > 0.5 ? to : 1
                return (
                  <div key={i} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '20px 0', borderTop: i ? `1px solid ${C.pRule}` : undefined }}>
                    <span style={{ fontSize: 29 }}>{label}</span>
                    <div style={{ position: 'relative', display: 'flex', background: 'rgba(27,28,26,0.06)', borderRadius: 12, padding: 4 }}>
                      <div style={{ position: 'absolute', top: 4, bottom: 4, left: 4 + pos * 112, width: 112, borderRadius: 9, background: TINT[ink], boxShadow: 'inset 0 0 0 1px rgba(27,28,26,0.06)' }} />
                      {CHOICES.map((ch, j) => <div key={j} style={{ position: 'relative', width: 112, textAlign: 'center', padding: '9px 0', fontSize: 23, fontWeight: j === ink ? 600 : 400, color: j === ink ? TINT_INK[ink] : C.pInk3 }}>{ch}</div>)}
                    </div>
                  </div>
                )
              })}
            </Paper>
          </div>
        )}
      </div>
    </AbsoluteFill>
  )
}

// ---------------- Skills: it learns what worked ----------------

const STEPS = ['Open playtomic.io, Sea Point Padel', 'Pick the day asked for, usually Thursday', 'Choose 18:30, Court 2, 90 minutes', 'Ask before booking', 'Reply with the confirmation']

export function Skills() {
  const t = useBeat()
  const c = cues('skills')
  const card = p(t, c.card, 1.2)
  const accepted = t >= c.accept
  const steps = [c.st1, c.st2, c.st3, c.st4, c.st5]
  const cur = glide(t, c.st5, c.accept - c.st5 - 0.35, [900, 700], [690, 578])
  const W = 800
  return (
    <AbsoluteFill style={{ flexDirection: 'row', alignItems: 'center', padding: '0 120px 0 150px', gap: 110 }}>
      <div style={{ position: 'relative', width: W, height: 660, opacity: card, translate: `0 ${(1 - card) * 50}px` }}>
        <Paper style={{ width: W, height: 660, padding: '38px 44px', display: 'flex', flexDirection: 'column' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12, ...eyebrow, color: accepted ? '#2B8E59' : C.pInk3, fontSize: 18 }}>
            <Icon name="wand" size={22} />{accepted ? 'Skill · active' : 'Skill · proposed'}
          </div>
          <div style={{ fontFamily: F.serif, fontSize: 56, lineHeight: 1.05, marginTop: 12 }}>Book usual padel court</div>
          <div style={{ fontFamily: F.mono, fontSize: 18, color: C.pInk3, marginTop: 10 }}>learned from a task that went well</div>
          <div style={{ marginTop: 22 }}>
            {STEPS.map((s, i) => {
              const k = p(t, steps[i], 0.6)
              return (
                <Grow key={i} k={p(t, steps[i] - 0.1, 0.5, ease.out)}>
                  <div style={{ display: 'flex', gap: 20, alignItems: 'baseline', padding: '12px 0', opacity: k, translate: `${(1 - k) * 20}px 0` }}>
                    <span style={{ fontFamily: F.serif, fontSize: 34, color: '#6E5135', width: 28 }}>{i + 1}</span>
                    <span style={{ fontSize: 28, lineHeight: 1.35 }}>{s}</span>
                  </div>
                </Grow>
              )
            })}
          </div>
          <div style={{ marginTop: 'auto', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 8 }}>
            {accepted ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, scale: 0.9 + 0.1 * p(t, c.accept, 0.5, ease.back) }}>
                <span style={{ fontSize: 24, color: C.pInk3 }}>Run it with</span>
                <span style={{ fontFamily: F.mono, fontSize: 26, padding: '9px 16px', borderRadius: 10, background: 'rgba(27,28,26,0.07)' }}>/book-padel</span>
              </div>
            ) : (
              <><div style={btnQuiet}>Dismiss</div><div style={{ ...btnInk, scale: 1 - press(t, c.accept) * 0.06 }}>Accept</div></>
            )}
          </div>
        </Paper>
        {t < c.accept + 0.8 && <Cursor x={cur.x} y={cur.y} press={press(t, c.accept)} />}
      </div>
      <div style={{ flex: 1 }}>
        <Side kicker="Skills" title={'It learns\nwhat worked.'} at={c.head} gl="From a task that went well, from your corrections, or from a link." glAt={c.st2} />
        <div style={{ marginTop: 44, width: 640, opacity: p(t, c.cmd - 0.5, 0.6) }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '14px 16px 14px 18px', borderRadius: 24, background: '#1F1F22', boxShadow: `inset 0 0 0 1px ${C.line}`, fontSize: 27, color: C.text }}>
            <Face id={BOT.ada} size={42} moods={[[0, 'listening'], [c.run, 'writing'], [c.run + 1.2, 'happy']]} />
            <span style={{ flex: 1, fontFamily: F.mono }}>{t < c.run ? <Typed text="/book-padel" at={c.cmd} dur={0.75} /> : <span style={{ color: C.text3, fontFamily: F.sans }}>Message Ada</span>}</span>
          </div>
          <Grow k={p(t, c.run + 0.3, 0.7)}>
            <div style={{ paddingTop: 16, fontSize: 28, color: C.text, fontFamily: F.sans }}>
              <span style={{ color: ADA, fontWeight: 600 }}>Ada</span>{'  '}<Stream text="Booked: Thursday 18:30, Court 2." at={c.run + 0.5} dur={1} />
            </div>
          </Grow>
        </div>
      </div>
    </AbsoluteFill>
  )
}

// ---------------- Bots: several bots, one memory ----------------

const CREW: [number, string, string][] = [
  [BOT.ada, 'Ada', 'Telegram'], [BOT.researcher, 'Researcher', 'reads the web'], [BOT.briefer, 'Morning briefer', '7:30 daily'],
  [BOT.reviewer, 'Weekly reviewer', 'Sundays'], [BOT.watcher, 'Page watcher', 'checks hourly'],
]

export function Bots() {
  const t = useBeat()
  const c = cues('bots')
  const at = [c.f1, c.f2, c.f3, c.f4, c.f5]
  const well: [number, number] = [960, 935]
  const xs = CREW.map((_, i) => 960 + (i - 2) * 330)
  const y0 = 600
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', top: 120, left: 0, right: 0, display: 'flex', justifyContent: 'center', gap: '0.3em', ...headline }}>
        <Words text="Several bots." at={c.head} step={0.25} />
        <Words text="*One* *memory.*" at={c.one} step={0.3} />
      </div>
      <svg width={1920} height={1080} style={{ position: 'absolute', inset: 0 }}>
        <ellipse cx={well[0]} cy={well[1]} rx={190} ry={36} fill="rgba(10,7,5,0.55)" stroke={C.foam} strokeOpacity={0.22 * p(t, c.flow - 0.5, 1)} strokeWidth={2} />
        {xs.map((x, i) => {
          const d = `M${x} ${y0 + 110} C ${x} ${y0 + 230}, ${well[0]} ${well[1] - 140}, ${well[0]} ${well[1]}`
          const k = p(t, c.flow + i * 0.08, 1, ease.io)
          const u = p(t, c.flow + 0.4 + i * 0.08, 1.2, ease.in)
          const bz = (a: number, b: number, cc: number, dd: number, s: number) => (1 - s) ** 3 * a + 3 * (1 - s) ** 2 * s * b + 3 * (1 - s) * s * s * cc + s ** 3 * dd
          const dx = bz(x, x, well[0], well[0], u), dy = bz(y0 + 110, y0 + 230, well[1] - 140, well[1], u)
          return (
            <g key={i}>
              <path d={d} fill="none" stroke={pigment(CREW[i][0])} strokeOpacity={0.55} strokeWidth={3} pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - k} />
              {u > 0 && u < 1 && <circle cx={dx} cy={dy} r={9} fill={pigment(CREW[i][0])} />}
            </g>
          )
        })}
      </svg>
      <Ripple x={well[0]} y={well[1]} at={c.flow + 1.5} size={700} rings={3} strength={0.3} beats={2.5} />
      {CREW.map(([id, name, role], i) => {
        const k = p(t, at[i], 0.7, ease.back)
        return (
          <div key={i} style={{ position: 'absolute', left: xs[i], top: y0 - 190, translate: '-50% 0', display: 'flex', flexDirection: 'column', alignItems: 'center', opacity: Math.min(1, k * 2), scale: 0.6 + 0.4 * k }}>
            <Face id={id} size={176} moods={[[0, 'happy'], [at[i] + 1.4, 'idle'], [c.flow, 'remembering']]} />
            <div style={{ fontFamily: F.serif, fontSize: 38, color: pigment(id), marginTop: 18, whiteSpace: 'nowrap', filter: 'brightness(1.25)' }}>{name}</div>
            <div style={{ fontFamily: F.mono, fontSize: 19, color: C.ink3, marginTop: 6 }}>{role}</div>
          </div>
        )
      })}
    </AbsoluteFill>
  )
}

// ---------------- Meetings: it takes the notes ----------------

const LINES: [string, string, string][] = [['Thandi', '09:02', 'Let’s ship on Thursday.'], ['Sam', '09:03', 'I’ll update the docs before then.'], ['You', '09:03', 'Great. Ada, write it up.']]

export function Meetings() {
  const t = useBeat()
  const c = cues('meetings')
  const card = p(t, c.card, 1.2)
  const at = [c.l1, c.l2, c.l3]
  const speaker = at.filter((x) => t >= x - 0.3).length - 1
  const notes = p(t, c.notes, 1, ease.out)
  const secs = 724 + Math.floor((t * 60) / 100)
  return (
    <AbsoluteFill style={{ flexDirection: 'row', alignItems: 'center', padding: '0 0 0 150px', gap: 100 }}>
      <Side kicker="Meetings & calls" title={'It takes\nthe notes.'} at={c.head} gl="Teams, Zoom, Meet and phone calls, transcribed on your computer. No bot joins." glAt={c.gloss} />
      <div style={{ position: 'relative', width: 920, height: 720 }}>
        <div style={{ opacity: card, translate: `0 ${(1 - card) * 50}px` }}>
          <Panel style={{ width: 900, padding: '28px 34px 30px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontSize: 24 }}>
              <span style={{ width: 14, height: 14, borderRadius: 7, background: C.danger, opacity: 0.55 + 0.45 * Math.cos(t * Math.PI) ** 2 }} />
              <span style={{ fontWeight: 600 }}>Recording</span><span style={{ color: C.text3 }}>· Standup</span>
              <span style={{ marginLeft: 'auto', fontFamily: F.mono, color: C.text3 }}>{Math.floor(secs / 60)}:{String(secs % 60).padStart(2, '0')}</span>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 5, height: 120, marginTop: 20 }}>
              {Array.from({ length: 76 }, (_, i) => {
                const n = Math.floor(t * 8) + i
                const amp = 0.18 + 0.82 * random(`w${n}`) * (0.55 + 0.45 * Math.sin(n * 0.37))
                return <span key={i} style={{ flex: 1, height: `${Math.max(6, amp * 100)}%`, borderRadius: 3, background: i > 62 ? C.text : C.text3, opacity: i > 62 ? 0.9 : 0.4 + (i / 76) * 0.4 }} />
              })}
            </div>
            <div style={{ marginTop: 20 }}>
              {LINES.map(([who, when, text], i) => (
                <Grow key={i} k={p(t, at[i], 0.6)}>
                  <div style={{ display: 'flex', gap: 20, padding: '14px 0', borderTop: `1px solid ${C.line}`, opacity: p(t, at[i], 0.8) * (i === speaker ? 1 : 0.6) }}>
                    <span style={{ width: 120, flex: 'none', fontSize: 22, color: C.text2, fontWeight: 600 }}>{who}<span style={{ display: 'block', fontFamily: F.mono, fontSize: 16, color: C.text3, fontWeight: 400, marginTop: 4 }}>{when}</span></span>
                    <span style={{ fontSize: 29, lineHeight: 1.35 }}>{text}</span>
                  </div>
                </Grow>
              ))}
            </div>
          </Panel>
        </div>
        {notes > 0 && (
          <div style={{ position: 'absolute', right: -40, bottom: -30, opacity: notes, translate: `${(1 - notes) * 40}px ${(1 - notes) * 60}px`, rotate: `${-1.8 + (1 - notes) * 4}deg` }}>
            <Paper style={{ width: 560, padding: '26px 30px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
                <Face id={BOT.ada} size={56} moods={[[0, 'writing'], [c.notes + 2, 'happy']]} />
                <div style={{ fontFamily: F.serif, fontSize: 38 }}>Notes · Standup</div>
              </div>
              <div style={{ fontSize: 26, lineHeight: 1.5, marginTop: 14 }}>
                <div><b style={{ fontWeight: 600 }}>Decided:</b> ship on Thursday.</div>
                <div><b style={{ fontWeight: 600 }}>To do:</b> Sam updates the docs.</div>
              </div>
            </Paper>
          </div>
        )}
      </div>
    </AbsoluteFill>
  )
}
