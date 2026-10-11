// Part two: things to try once it's set up. Each scene is one thing you say, and what you get back.
import type { CSSProperties, ReactNode } from 'react'
import { AbsoluteFill } from 'remotion'
import { BOT, C, cues, ease, eyebrow, F, Face, gloss, Grow, headline, Icon, p, Panel, Paper, pigment, Ripple, Rise, Stream, Typed, useBeat, Words } from '../lib'
import { Bot, Bubble, Card, Composer, Enter, Gloss, Head, near, OK, Phone, Pointer, Tag, Win, You } from './kit'

export function Daily() {
  const c = cues('daily')
  return <Card kicker="Part two" title="Then, every day." gl="Things to try. Just say them." glAt={c.gloss} at={c.head} />
}

/** The app's chat with Ada, as one window: header, messages from the bottom up, the message box. */
function Chat({ children, composer, sub = 'Built-in chat', style }: { children: ReactNode; composer?: ReactNode; sub?: string; style?: CSSProperties }) {
  return (
    <Win style={{ height: '100%', ...style }}>
      <div style={{ position: 'absolute', left: 36, right: 36, top: 20, display: 'flex', alignItems: 'center', gap: 16, paddingBottom: 16, borderBottom: `1px solid ${C.line}` }}>
        <Face id={BOT.ada} size={48} />
        <div><div style={{ fontFamily: F.ui, fontSize: 27, fontWeight: 500 }}>Ada</div><div style={{ fontSize: 18, color: C.text3 }}>{sub}</div></div>
      </div>
      <div style={{ position: 'absolute', left: 40, right: 40, top: 104, bottom: composer ? 104 : 24, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', overflow: 'hidden', maskImage: 'linear-gradient(transparent, #000 70px)' }}>{children}</div>
      {composer && <div style={{ position: 'absolute', left: 30, right: 30, bottom: 22 }}>{composer}</div>}
    </Win>
  )
}
const faded: CSSProperties = { opacity: 0.4 }

// ---------------- Ask: it remembers, and says what it used ----------------

export function Ask() {
  const t = useBeat()
  const c = cues('ask')
  const open = p(t, c.open, 0.8)
  return (
    <AbsoluteFill>
      <Head title="Ask. It remembers." at={c.head} />
      <Enter at={-1} x={360} y={250} w={1200} h={620}>
        <Chat composer={<Composer text="Who's my dentist?" at={c.type} dur={0.75} send={c.send} />}>
          <div style={faded}>
            <You at={-9}>Remember that my dentist is Dr. Lee in Stellenbosch.</You>
            <Bot at={-9} moods={[[0, 'idle']]}><span>Saved: Dr. Lee in Stellenbosch.</span></Bot>
          </div>
          <div style={{ textAlign: 'center', fontSize: 18, color: C.text3, padding: '4px 0 18px' }}>Two weeks later</div>
          <You at={c.send}>Who’s my dentist?</You>
          <Bot at={c.reply} text="Dr. Lee, in Stellenbosch. Want me to find a time for your check-up?" dur={1.4} wait="Searching memory"
            moods={[[0, 'idle'], [c.send + 0.2, 'searching'], [c.reply, 'writing'], [c.reply + 1.7, 'idle']]}
            below={
              <>
                <Grow k={p(t, c.used, 0.5)}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, paddingTop: 12, fontSize: 20, color: open > 0 ? C.text2 : C.text3, opacity: p(t, c.used, 0.6) }}>
                    <Icon name="memory" size={19} />1 memory · Dentist<span style={{ display: 'flex', rotate: `${open * 90}deg` }}><Icon name="chevron" size={18} /></span>
                  </div>
                </Grow>
                <Grow k={open}>
                  <div style={{ paddingTop: 14, opacity: open }}>
                    <Paper style={{ width: 600, padding: '18px 24px 16px' }}>
                      <div style={{ fontFamily: F.serif, fontSize: 32, lineHeight: 1.05 }}>Dentist</div>
                      <div style={{ fontSize: 23, color: C.pInk2, marginTop: 6 }}>Dr. Lee in Stellenbosch.</div>
                      <div style={{ fontFamily: F.mono, fontSize: 16, color: C.pInk3, display: 'flex', gap: 18, marginTop: 10 }}><span>you said</span><span>12 Sep</span><span>used 4 times</span></div>
                    </Paper>
                  </div>
                </Grow>
              </>
            } />
        </Chat>
      </Enter>
      <Gloss at={c.gloss}>Under every reply, a quiet line says what it used.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Plan: a schedule in plain words, or off your calendar ----------------

function Cal({ size = 64 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" style={{ flex: 'none' }}>
      <rect x="6" y="11" width="52" height="47" rx="9" fill="none" stroke={C.pInk} strokeOpacity={0.55} strokeWidth="3.5" />
      <path d="M6 25h52M20 5v12M44 5v12" stroke={C.pInk} strokeOpacity={0.55} strokeWidth="3.5" strokeLinecap="round" />
      <rect x="15" y="33" width="10" height="9" rx="2" fill={pigment(BOT.ada)} />
    </svg>
  )
}
function Clock({ size = 64 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="-32 -32 64 64" style={{ flex: 'none' }}>
      <circle r="27" fill="none" stroke={C.pInk} strokeOpacity={0.55} strokeWidth="3.5" />
      <path d="M0 0V-16M0 0L11 7" stroke={C.pInk} strokeWidth="4" strokeLinecap="round" />
    </svg>
  )
}

function Routine({ k, said, icon, when, what, next }: { k: number; said: string; icon: ReactNode; when: string; what: string; next: string }) {
  return (
    <div style={{ opacity: Math.min(1, k * 1.5), translate: `0 ${(1 - k) * 60}px`, scale: 0.94 + 0.06 * k }}>
      <Paper style={{ padding: '26px 34px 28px' }}>
        <div style={{ fontFamily: F.serif, fontStyle: 'italic', fontSize: 26, color: C.pInk2, lineHeight: 1.3 }}>“{said}”</div>
        <div style={{ height: 1, background: C.pRule, margin: '20px 0 22px' }} />
        <div style={{ display: 'flex', alignItems: 'center', gap: 24 }}>
          {icon}
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: F.serif, fontSize: 44, lineHeight: 1.05 }}>{when}</div>
            <div style={{ fontSize: 24, color: C.pInk2, marginTop: 6 }}>{what}</div>
          </div>
          <Face id={BOT.ada} size={64} moods={[[0, 'happy'], [3, 'idle']]} />
        </div>
        <div style={{ fontFamily: F.mono, fontSize: 18, color: C.pInk3, marginTop: 18, letterSpacing: '0.02em' }}>{next}</div>
      </Paper>
    </div>
  )
}

const SAY1 = 'Every weekday at 7:30, brief me on tech news in 3 bullets.'
const SAY2 = 'An hour before each meeting, tell me who’s coming.'

export function Plan() {
  const t = useBeat()
  const c = cues('plan')
  const second = t >= c.send1 + 1
  return (
    <AbsoluteFill>
      <Head title="Give it a schedule." at={c.head} />
      <div style={{ position: 'absolute', left: 360, top: 262, width: 1200 }}>
        {second
          ? <Composer text={SAY2} at={c.type2} dur={1.75} send={c.send2} size={28} />
          : <Composer text={SAY1} at={c.type1} dur={2} send={c.send1} size={28} />}
      </div>
      <div style={{ position: 'absolute', left: 190, top: 470, width: 740 }}>
        <Routine k={p(t, c.card1, 1, ease.out)} said={SAY1} icon={<Clock />} when="Weekdays at 7:30 AM" what="Brief me on tech news, in 3 bullets" next="Next: tomorrow 7:30 · in Telegram" />
      </div>
      <div style={{ position: 'absolute', left: 990, top: 470, width: 740 }}>
        <Routine k={p(t, c.card2, 1, ease.out)} said={SAY2} icon={<Cal />} when="1 hour before each meeting" what="Tell me who’s coming" next="Off your calendar · next: Tue 8:30, for Standup" />
      </div>
      <Gloss at={c.gloss}>In plain words. Or timed off your calendar.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Watch: a quiet check, free while nothing changes ----------------

const RUNS: [string, string][] = [['r1', '09:00'], ['r2', '10:00'], ['r3', '11:00'], ['change', '12:00']]

export function Watch() {
  const t = useBeat()
  const c = cues('watch')
  const card = p(t, c.card, 1)
  const note = p(t, c.change + 0.3, 0.9)
  const say = '“Keep an eye on example.com/pricing. Only tell me if it changes.”'
  return (
    <AbsoluteFill>
      <Head title="Or have it keep watch." at={c.head} size={84} />
      <div style={{ position: 'absolute', top: 236, left: 0, right: 0, display: 'flex', justifyContent: 'center' }}>
        <div style={{ ...headline, fontStyle: 'italic', fontSize: 50, color: C.gall, position: 'relative' }}>
          <span style={{ opacity: 0 }}>{say}</span>
          <span style={{ position: 'absolute', inset: 0 }}><Typed text={say} at={c.type} dur={1.5} caret={t < c.card} /></span>
        </div>
      </div>
      <div style={{ position: 'absolute', left: 410, top: 370, width: 1100, opacity: card, translate: `0 ${(1 - card) * 50}px` }}>
        <Paper style={{ padding: '30px 38px 32px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 22 }}>
            <Face id={BOT.watcher} size={76} moods={[[0, 'idle'], [c.r1 - 0.2, 'reading'], [c.r3 + 0.6, 'idle'], [c.change - 0.2, 'attention']]} />
            <div style={{ flex: 1 }}>
              <div style={{ ...eyebrow, fontSize: 17, color: C.pInk3 }}>Quiet check · every hour</div>
              <div style={{ fontFamily: F.serif, fontSize: 46, lineHeight: 1.1, marginTop: 4 }}>example.com/pricing</div>
            </div>
            <div style={{ fontFamily: F.mono, fontSize: 19, color: C.pInk3, textAlign: 'right' }}>AI used<br /><b style={{ fontSize: 26, color: t >= c.change ? C.pInk : OK }}>{t >= c.change ? '1 run' : '0 tokens'}</b></div>
          </div>
          <div style={{ display: 'flex', gap: 16, marginTop: 26 }}>
            {RUNS.map(([cue, time], i) => {
              const k = p(t, c[cue], 0.5, ease.back)
              const hit = i === 3
              return (
                <div key={cue} style={{ flex: 1, padding: '16px 18px', borderRadius: 12, background: hit ? 'rgba(226,166,71,0.16)' : 'rgba(27,28,26,0.05)', opacity: Math.min(1, k * 1.5), scale: 0.9 + 0.1 * k }}>
                  <div style={{ fontFamily: F.mono, fontSize: 21, color: C.pInk3 }}>{time}</div>
                  <div style={{ fontSize: 23, marginTop: 4, color: hit ? '#9A6409' : C.pInk2, fontWeight: hit ? 600 : 400 }}>{hit ? 'Changed' : 'No change'}</div>
                  <div style={{ fontSize: 18, color: hit ? '#9A6409' : OK, marginTop: 2 }}>{hit ? 'Pro: €15 → €12' : 'free, no AI'}</div>
                </div>
              )
            })}
          </div>
        </Paper>
      </div>
      {note > 0 && (
        <div style={{ position: 'absolute', left: 960, top: 760, translate: `-50% ${(1 - note) * 40}px`, opacity: note }}>
          <Panel style={{ width: 780, borderRadius: 26, padding: '20px 26px', display: 'flex', gap: 20, alignItems: 'center', background: 'rgba(44,44,49,0.97)' }}>
            <Face id={BOT.watcher} size={60} moods={[[0, 'writing']]} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 22 }}><b style={{ fontWeight: 600 }}>Page watcher</b><span style={{ color: C.text3, fontFamily: F.mono }}>12:00</span></div>
              <div style={{ fontSize: 23, color: C.text2, marginTop: 3 }}>Pricing changed: Pro is now €12 a month (was €15).</div>
            </div>
          </Panel>
        </div>
      )}
      <Gloss at={c.gloss}>It looks first, and only wakes the AI when something changed.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Tap: buttons under reminders and results ----------------

const KEYS = ['Done', 'Snooze 1h', 'Tomorrow']

export function Tap() {
  const t = useBeat()
  const c = cues('tap')
  const used = t >= c.tap
  const ring = p(t, c.tap, 0.8, ease.out)
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', left: 150, top: 290, width: 860 }}>
        <Rise at={c.head} y={20}><div style={{ ...eyebrow, marginBottom: 26 }}>In Telegram, Slack and Discord</div></Rise>
        <Words text={'Tap instead\nof typing.'} at={c.head + 0.15} step={0.2} style={{ ...headline, fontSize: 112 }} />
        <Rise at={c.gloss} style={{ marginTop: 40 }}><p style={{ ...gloss, maxWidth: 800 }}>Reminders come with Done, Snooze and Tomorrow. Routine results with Like, Less like this and Pause.</p></Rise>
      </div>
      <div style={{ position: 'absolute', left: 1240, top: 110 }}>
        <Phone k={p(t, c.phone, 1.1)} clock="15:40" moods={[[0, 'idle'], [c.msg - 0.4, 'attention'], [c.done, 'happy'], [c.done + 1.5, 'idle']]}>
          <div style={faded}>
            <Bubble at={-9}>Three things in tech today. Rust 1.96 is out, and…</Bubble>
          </div>
          <Bubble at={c.msg}>Reminder: call mum.</Bubble>
          <Grow k={p(t, c.msg + 0.2, 0.6)}>
            <div style={{ display: 'flex', gap: 6, paddingTop: 6, paddingLeft: 46, opacity: used ? 1 - 0.55 * p(t, c.done, 0.5) : 1 }}>
              {KEYS.map((k, i) => (
                <span key={k} style={{ position: 'relative', flex: 1, textAlign: 'center', padding: '11px 4px', borderRadius: 12, fontSize: 19, fontWeight: 600, color: C.text, background: i === 1 && used ? 'rgba(255,255,255,0.22)' : 'rgba(255,255,255,0.1)', overflow: 'hidden', scale: i === 1 ? 1 - near(t, c.tap, 0.15) * 0.06 : 1 }}>
                  {k}
                  {i === 1 && ring > 0 && ring < 1 && <span style={{ position: 'absolute', left: '50%', top: '50%', width: 160, height: 160, translate: '-50% -50%', borderRadius: '50%', background: 'rgba(255,255,255,0.35)', scale: ring, opacity: 1 - ring }} />}
                </span>
              ))}
            </div>
          </Grow>
          <Bubble at={c.done}>Snoozed. I’ll remind you at 16:40.</Bubble>
        </Phone>
      </div>
    </AbsoluteFill>
  )
}

// ---------------- Okay: it asks first, then a receipt ----------------

const DRAFT = 'Hi, I’ll pay invoice 4411 on Friday. Thanks!'
const ALLOW = { x: 1664, y: 558 }

export function Okay() {
  const t = useBeat()
  const c = cues('okay')
  const card = p(t, c.card, 1)
  const sent = t >= c.sent
  const away = p(t, c.sent + 0.6, 0.8, ease.in)
  return (
    <AbsoluteFill>
      <Head title="It asks before it acts." at={c.head} />
      <Enter at={-1} x={150 + 380 * p(t, c.sent + 0.8, 1.4, ease.io)} y={250} w={860} h={640}>
        <Chat composer={<Composer text="Reply to Acme that I’ll pay on Friday." at={c.type} dur={1.5} send={c.send} />}>
          <You at={c.send}>Reply to Acme that I’ll pay on Friday.</You>
          <Bot at={c.sent + 0.25} from={c.send + 0.4} text="Sent." dur={0.4} wait="Waiting for your OK"
            moods={[[0, 'idle'], [c.send + 0.2, 'writing'], [c.card, 'asking'], [c.sent, 'happy'], [c.sent + 1.5, 'idle']]}
            below={
              <Grow k={p(t, c.receipt, 0.5)}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8, paddingTop: 12 }}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 21, color: C.live, opacity: p(t, c.receipt, 0.5) }}><Icon name="check" size={20} />Sent an email to accounts@acme.example</span>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 19, color: C.text3, opacity: p(t, c.receipt + 0.3, 0.5) }}><Icon name="inbox" size={18} />In your Sent folder, as if you’d sent it</span>
                </div>
              </Grow>
            } />
        </Chat>
      </Enter>
      {card > 0 && away < 1 && (
        <div style={{ position: 'absolute', left: 1070, top: 290, width: 700, opacity: card * (1 - away), translate: `${-away * 300}px ${(1 - card) * 40 + away * 120}px`, scale: 1 - away * 0.4 }}>
          <Paper style={{ padding: '32px 36px 30px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 20 }}>
              <Face id={BOT.ada} size={76} moods={[[0, 'asking'], [c.sent, 'happy']]} />
              <div>
                <div style={{ fontFamily: F.serif, fontSize: 40, lineHeight: 1.05 }}><span style={{ color: pigment(BOT.ada) }}>Ada</span> wants to send an email</div>
                <div style={{ fontSize: 21, color: C.pInk3, marginTop: 6 }}>To accounts@acme.example · Re: Invoice 4411</div>
              </div>
            </div>
            <div style={{ marginTop: 24, padding: '20px 24px', borderRadius: 12, background: 'rgba(27,28,26,0.05)', fontSize: 27, lineHeight: 1.45 }}>{DRAFT}</div>
            <div style={{ marginTop: 26, display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 8, height: 54 }}>
              {sent ? (
                <div style={{ fontSize: 24, padding: '12px 26px', borderRadius: 12, background: OK, color: C.paper, display: 'flex', alignItems: 'center', gap: 10, scale: 0.9 + 0.1 * p(t, c.sent, 0.5, ease.back) }}><Icon name="check" size={24} />Sent</div>
              ) : (
                <>
                  <div style={{ fontSize: 24, padding: '12px 18px', color: C.pInk2 }}>Deny</div>
                  <div style={{ fontSize: 24, padding: '12px 26px', borderRadius: 12, background: t > c.allow - 0.6 ? '#33342F' : C.pInk, color: C.paper, scale: 1 - near(t, c.allow, 0.15) * 0.06 }}>Allow once</div>
                </>
              )}
            </div>
          </Paper>
        </div>
      )}
      <Pointer ways={[[c.cursor, 1560, 1010], [c.allow - 0.2, ALLOW.x, ALLOW.y], [c.allow + 0.5, ALLOW.x, ALLOW.y], [c.allow + 1.3, 1720, 900]]} clicks={[c.allow]} to={c.allow + 1.4} />
      <Gloss at={c.gloss}>Sending, booking, deleting: it asks first. Then a receipt from the log.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Quick: from any app ----------------

function Key({ children, w = 96, down }: { children: ReactNode; w?: number; down: number }) {
  return (
    <span style={{ width: w, height: 96, borderRadius: 18, display: 'grid', placeItems: 'center', background: 'linear-gradient(#3A3A3F, #29292D)', color: C.text, fontSize: 34, fontWeight: 500,
      boxShadow: `0 ${8 - down * 6}px 0 #1A1A1C, 0 18px 30px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.12)`, translate: `0 ${down * 6}px` }}>{children}</span>
  )
}

export function Quick() {
  const t = useBeat()
  const c = cues('quick')
  const keys = p(t, c.keys - 0.6, 0.5) * (1 - p(t, c.panel, 0.5, ease.in))
  const down = t >= c.keys - 0.1 && t < c.keys + 0.35 ? 1 : 0
  const panel = p(t, c.panel, 0.7, ease.back)
  return (
    <AbsoluteFill>
      <Head title="Ask from any app." at={c.head} />
      <div style={{ position: 'absolute', left: 360, top: 250, width: 1200, height: 640, opacity: 0.5, filter: panel > 0 ? `blur(${panel * 3}px)` : undefined }}>
        <Win title="Q4 plan — Notes" style={{ height: '100%', background: '#232326' }}>
          <div style={{ position: 'absolute', left: 70, top: 50, right: 70 }}>
            <div style={{ fontFamily: F.ui, fontSize: 40, color: C.text, marginBottom: 30 }}>Q4 plan</div>
            {[92, 84, 96, 60, 0, 88, 94, 70, 0, 80, 66].map((w, i) => <div key={i} style={{ height: 14, width: `${w}%`, borderRadius: 7, background: 'rgba(255,255,255,0.1)', marginBottom: 22 }} />)}
          </div>
        </Win>
      </div>
      {keys > 0 && (
        <div style={{ position: 'absolute', left: 960, top: 520, translate: '-50% 0', display: 'flex', alignItems: 'center', gap: 22, opacity: keys, scale: 0.9 + 0.1 * keys }}>
          <Key down={down}>⌥</Key><span style={{ fontSize: 40, color: C.ink3 }}>+</span><Key w={300} down={down}>Space</Key>
        </div>
      )}
      {panel > 0 && (
        <div style={{ position: 'absolute', left: 960, top: 360, translate: '-50% 0', opacity: Math.min(1, panel * 1.5), scale: 0.92 + 0.08 * panel }}>
          <Panel style={{ width: 960, borderRadius: 26, background: 'rgba(40,40,44,0.98)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 20, padding: '22px 28px' }}>
              <Face id={BOT.ada} size={56} moods={[[0, 'listening'], [c.type + 1.2, 'thinking'], [c.reply, 'writing'], [c.reply + 1.6, 'idle']]} />
              <div style={{ flex: 1, fontSize: 32, color: t >= c.type ? C.text : C.text3 }}>{t >= c.type ? <Typed text="What’s on tomorrow?" at={c.type} dur={1} caret={t < c.reply} /> : 'Ask Ada'}</div>
              <span style={{ fontFamily: F.mono, fontSize: 18, color: C.text3 }}>esc</span>
            </div>
            <Grow k={p(t, c.reply - 0.3, 0.6)}>
              <div style={{ borderTop: `1px solid ${C.line}`, padding: '22px 28px 26px 104px' }}>
                <div style={{ fontSize: 28, lineHeight: 1.45 }}><Stream text="Standup at 9:30, then the dentist at 10 with Dr. Lee." at={c.reply} dur={1.2} /></div>
                <div style={{ paddingTop: 14 }}><Tag icon="clock" k={p(t, c.reply + 1.3, 0.5)}>Calendar · Tue 14 Oct</Tag></div>
              </div>
            </Grow>
          </Panel>
        </div>
      )}
      <Gloss at={c.gloss}>Option-Space on a Mac, Ctrl+Alt+Space on Windows and Linux.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Correct: say when it's wrong; it learns once you accept ----------------

const NEWS = ['Rust 1.96 is out, with faster builds', 'A new open model tops the coding charts', 'Apple ships a beta of macOS 27.1', 'EU AI rules: the next deadline moves', 'A big outage at a cloud provider', 'Three startups raise for AI agents']
const ACCEPT = { x: 1434, y: 862 }

export function Correct() {
  const t = useBeat()
  const c = cues('correct')
  const sugg = p(t, c.sugg, 0.9)
  const accepted = t >= c.accept
  return (
    <AbsoluteFill>
      <Head title="Say when it’s wrong." at={c.head} />
      <Enter at={-1} x={360} y={240} w={1200} h={540}>
        <Chat sub="Telegram">
          <div style={faded}>
            <Bot at={-9} moods={[[0, 'idle']]}>
              <div style={{ fontSize: 23, lineHeight: 1.5 }}>Tech news this morning:{NEWS.map((n) => <div key={n}>• {n}</div>)}</div>
            </Bot>
          </div>
          <You at={c.send}>Too long. Three bullets, please.</You>
          <Bot at={c.reply} text="Got it: three bullets from now on." dur={0.8} moods={[[0, 'idle'], [c.send + 0.1, 'listening'], [c.reply, 'writing'], [c.reply + 1, 'remembering'], [c.accept, 'happy']]} />
        </Chat>
      </Enter>
      {sugg > 0 && (
        <div style={{ position: 'absolute', left: 410, top: 806, width: 1100, opacity: sugg, translate: `0 ${(1 - sugg) * 40}px` }}>
          <Paper style={{ display: 'flex', alignItems: 'center', gap: 22, padding: '18px 22px 18px 28px', background: C.paper2 }}>
            <Icon name="wand" size={30} />
            <div style={{ flex: 1 }}>
              <div style={{ fontFamily: F.mono, fontSize: 16, color: C.pInk3, letterSpacing: '0.12em', textTransform: 'uppercase' }}>Suggested by the tidy-up</div>
              <div style={{ fontSize: 26, marginTop: 4 }}>Tech news: three bullets, most important first.</div>
            </div>
            {accepted ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: OK, fontSize: 25, fontWeight: 600, padding: '0 14px', opacity: p(t, c.accept, 0.3) }}><Icon name="check" size={28} />Accepted</div>
            ) : (
              <>
                <div style={{ fontSize: 24, padding: '12px 20px', color: C.pInk2 }}>Dismiss</div>
                <div style={{ fontSize: 24, padding: '12px 24px', borderRadius: 12, background: C.pInk, color: C.paper, scale: 1 - near(t, c.accept, 0.15) * 0.06 }}>Accept</div>
              </>
            )}
          </Paper>
        </div>
      )}
      <Ripple x={ACCEPT.x} y={ACCEPT.y} at={c.accept} size={300} rings={2} strength={0.3} beats={1.5} />
      <Pointer ways={[[c.sugg + 0.4, 1600, 1030], [c.accept - 0.2, ACCEPT.x, ACCEPT.y + 4], [c.accept + 0.6, ACCEPT.x, ACCEPT.y + 4], [c.accept + 1.4, 1640, 1000]]} clicks={[c.accept]} to={c.accept + 1.5} />
      <Gloss at={c.gloss} bottom={36}>It learns from corrections. Nothing changes until you accept.</Gloss>
    </AbsoluteFill>
  )
}
