// Part one: what Mimir is in one breath, then setting it up in six steps, each as it looks in the app.
import type { CSSProperties, ReactNode } from 'react'
import { AbsoluteFill } from 'remotion'
import { BOT, C, cues, ease, eyebrow, F, Face, gloss, Grow, headline, Icon, lerp, p, Paper, pigment, Ripple, Rise, Typed, useBeat, Words } from '../lib'
import { Mark } from '../scenes/open'
import { Badge, Bot, Btn, Bubble, Card, Composer, Enter, Field, near, Phone, Pointer, StepSide, Tag, Win, You } from './kit'

// The app's window, where every setup step happens, beside the step's header.
const W = { x: 830, y: 150, w: 960, h: 820 }
const IN = { x: W.x + 40, y: W.y + 46 } // the window's content, inside its padding
const side: CSSProperties = { position: 'absolute', left: 150, top: 250 }

// ---------------- Intro: the mark, and what this is ----------------

export function Intro() {
  const t = useBeat()
  const c = cues('intro')
  const rune = [0, 1, 2, 3].map((i) => p(t, c.rune + i * 0.3, 0.8, ease.io))
  const w = p(t, c.word, 1.4)
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      <Ripple x={960} y={430} at={c.word} size={1300} rings={3} strength={0.22} beats={5} />
      <Mark id="gi" size={210} draw={p(t, c.draw, 2.2, ease.io)} fill={p(t, c.draw + 1, 1.8, ease.soft)} rune={rune} />
      <Grow k={p(t, c.word, 1.6, ease.io)}>
        <div style={{ textAlign: 'center', paddingTop: 40, opacity: w, filter: w < 1 ? `blur(${(1 - w) * 12}px)` : undefined, translate: `0 ${(1 - w) * 30}px` }}>
          <div style={{ ...eyebrow, fontSize: 28 }}>How to use</div>
          <div style={{ ...headline, fontSize: 168, letterSpacing: '0.01em', marginTop: 10 }}>Mimir</div>
        </div>
      </Grow>
      <Grow k={p(t, c.sub, 1.2, ease.io)}>
        <Words text="From download to your first routine, in two minutes." at={c.sub} step={0.18} style={{ ...gloss, fontSize: 46, paddingTop: 20, textAlign: 'center' }} />
      </Grow>
    </AbsoluteFill>
  )
}

// ---------------- What: what it is, in plain words ----------------

const WHAT: [string, string, string][] = [
  ['chat', 'l1', 'You message them in *Telegram,* *Slack,* *Discord* or the app.'],
  ['memory', 'l2', 'They remember you, keep a *schedule,* and use your *email,* *calendar* and *the* *web.*'],
  ['shield', 'l3', 'They run on the AI plan *you* *already* *pay* *for,* and ask before they act.'],
]

export function What() {
  const t = useBeat()
  const c = cues('what')
  return (
    <AbsoluteFill>
      <div style={{ position: 'absolute', top: 130, left: 0, right: 0, textAlign: 'center' }}>
        <Words text="Mimir runs AI assistants *for* *you.*" at={c.head} step={0.2} style={{ ...headline, fontSize: 100 }} />
      </div>
      <div style={{ position: 'absolute', top: 370, left: 230, display: 'flex', flexDirection: 'column', gap: 54 }}>
        {WHAT.map(([icon, cue, text]) => {
          const k = p(t, c[cue], 0.9, ease.back)
          return (
            <div key={cue} style={{ display: 'flex', alignItems: 'center', gap: 40 }}>
              <span style={{ width: 84, height: 84, flex: 'none', borderRadius: 42, display: 'grid', placeItems: 'center', color: C.gall, boxShadow: 'inset 0 0 0 2px rgba(223,193,154,0.45)', opacity: Math.min(1, k * 1.5), scale: 0.6 + 0.4 * k }}>
                <Icon name={icon} size={40} />
              </span>
              <Words text={text} at={c[cue] + 0.1} step={0.08} dur={0.9} style={{ fontFamily: F.serif, fontSize: 46, color: C.foam, lineHeight: 1.2 }} />
            </div>
          )
        })}
      </div>
      <div style={{ position: 'absolute', bottom: 80, left: 0, right: 0, display: 'flex', justifyContent: 'center', alignItems: 'center', gap: 28 }}>
        <div style={{ display: 'flex', gap: 6 }}>
          {[BOT.ada, BOT.researcher, BOT.briefer].map((id, i) => {
            const k = p(t, c.faces + i * 0.25, 0.7, ease.back)
            return <Face key={id} id={id} size={66} moods={[[0, 'happy']]} style={{ opacity: Math.min(1, k * 2), scale: 0.5 + 0.5 * k }} />
          })}
        </div>
        <Rise at={c.gloss + 0.3}><p style={{ ...gloss, fontSize: 40 }}>No Mimir account, no Mimir servers. Free.</p></Rise>
      </div>
    </AbsoluteFill>
  )
}

export function Setup() {
  const c = cues('setup')
  return <Card kicker="Part one" title="Set it up." gl="Six steps, about ten minutes." glAt={c.gloss} at={c.head} />
}

// ---------------- 1. Download ----------------

const DOCK = { x: 1060, y: 906, n: 6, s: 70, gap: 16 }
const slot = (i: number) => DOCK.x + 24 + i * (DOCK.s + DOCK.gap) + DOCK.s / 2
const APPS = ['#3D8DB5', '#5E8F45', '#BD8A2B', '#8160BE', '#B5527D']

/** The app's icon: the mark on its dark tile. */
function AppIcon({ size }: { size: number }) {
  return (
    <div style={{ width: size, height: size, borderRadius: size * 0.23, background: 'linear-gradient(#2B1F13, #0E0A06)', boxShadow: 'inset 0 0 0 1.5px rgba(255,240,220,0.12), 0 6px 14px rgba(0,0,0,0.5)', display: 'grid', placeItems: 'center' }}>
      <Mark id={`ai${size}`} size={size * 0.78} />
    </div>
  )
}

export function Get() {
  const t = useBeat()
  const c = cues('get')
  const dl = p(t, c.dl, 0.7, ease.back)
  const fly = p(t, c.fly, c.land - c.fly, ease.io)
  const land = t >= c.land
  const bounce = (at: number) => { const u = (t - at) / 0.9; return u > 0 && u < 1 ? Math.sin(u * Math.PI) : 0 }
  const hop = t >= c.open ? Math.max(bounce(c.open), bounce(c.open + 1)) : 0
  const from = { x: 1620, y: 282 }, to = { x: slot(5), y: DOCK.y + 42 }
  const btn = { x: 1310, y: 578 }
  return (
    <AbsoluteFill>
      <div style={side}><StepSide n={1} title={'Download it.'} gl="macOS, Windows and Linux. Signed, and it updates itself." glAt={c.gloss} /></div>
      <Enter at={c.win} x={W.x} y={190} w={W.w} h={560}>
        <Win title="heymimir.app" style={{ height: '100%' }}>
          <div style={{ position: 'absolute', inset: 0, background: `radial-gradient(ellipse 80% 90% at 50% 30%, #2A1D10, ${C.deep})`, display: 'flex', flexDirection: 'column', alignItems: 'center', paddingTop: 56 }}>
            <Mark id="gw" size={96} />
            <div style={{ fontFamily: F.serif, fontSize: 64, color: C.foam, marginTop: 18, lineHeight: 1 }}>Mimir</div>
            <div style={{ ...gloss, fontSize: 30, marginTop: 14 }}>Your own AI assistant, always on.</div>
            <Btn kind="primary" icon="down" press={near(t, c.click, 0.15)} hot={near(t, c.click)} size={26} style={{ marginTop: 34, padding: '16px 30px', background: C.foam, color: C.deep }}>Download for macOS</Btn>
            <div style={{ fontFamily: F.mono, fontSize: 18, color: C.ink3, marginTop: 18, letterSpacing: '0.06em' }}>Windows · Linux · all downloads</div>
          </div>
          {dl > 0 && (
            <div style={{ position: 'absolute', right: 22, top: 16, opacity: Math.min(1, dl * 1.5), scale: 0.8 + 0.2 * dl, display: 'flex', alignItems: 'center', gap: 12, padding: '10px 16px', borderRadius: 14, background: 'rgba(30,30,33,0.96)', boxShadow: `0 10px 30px rgba(0,0,0,0.5), inset 0 0 0 1px ${C.line}`, fontSize: 19, color: C.text }}>
              <span style={{ width: 40, height: 40, opacity: fly > 0 ? 0.25 : 1 }}><AppIcon size={40} /></span>
              <span style={{ fontFamily: F.mono }}>Mimir_0.1.5.dmg</span>
              <Badge live k={p(t, c.dl + 0.5, 0.5)}>Done</Badge>
            </div>
          )}
        </Win>
      </Enter>
      {/* the dock, and the app dropping into it */}
      <div style={{ position: 'absolute', left: DOCK.x, top: DOCK.y, width: DOCK.n * (DOCK.s + DOCK.gap) + 32, height: 96, borderRadius: 30, background: 'rgba(236,233,225,0.07)', boxShadow: 'inset 0 0 0 1px rgba(236,233,225,0.12)', opacity: p(t, c.dl, 1), translate: `0 ${(1 - p(t, c.dl, 1)) * 40}px` }}>
        {APPS.map((col, i) => <div key={i} style={{ position: 'absolute', left: slot(i) - DOCK.x - DOCK.s / 2, top: 13, width: DOCK.s, height: DOCK.s, borderRadius: 16, background: `linear-gradient(${col}, color-mix(in oklab, ${col} 60%, black))`, opacity: 0.55 }} />)}
        {land && <span style={{ position: 'absolute', left: slot(5) - DOCK.x - 3, bottom: 4, width: 6, height: 6, borderRadius: 3, background: C.foam, opacity: p(t, c.open, 0.5) * 0.8 }} />}
      </div>
      {fly > 0 && (
        <div style={{ position: 'absolute', left: lerp(from.x, to.x, fly) - lerp(20, 35, fly), top: lerp(from.y, to.y, fly) - Math.sin(fly * Math.PI) * 160 - lerp(20, 35, fly) - hop * 34, width: lerp(40, 70, fly), zIndex: 5 }}>
          <AppIcon size={lerp(40, 70, fly)} />
        </div>
      )}
      {land && <Ripple x={to.x} y={to.y} at={c.land} size={260} rings={2} strength={0.3} beats={1.5} />}
      {t >= c.open && (
        <div style={{ position: 'absolute', left: to.x, top: DOCK.y - 54, translate: '-50% 0', padding: '6px 14px', borderRadius: 8, background: 'rgba(30,30,33,0.9)', fontSize: 19, color: C.text, opacity: p(t, c.open, 0.4) }}>Mimir</div>
      )}
      <Pointer ways={[[1.5, 1560, 1010], [3.3, btn.x + 20, btn.y + 6], [4.3, btn.x + 20, btn.y + 6], [5.2, 1700, 820]]} clicks={[c.click]} to={5.4} />
    </AbsoluteFill>
  )
}

// ---------------- 2. Your AI ----------------

const ACCOUNTS: [string, string, string, string][] = [
  ['C', 'Claude', 'Your Claude subscription, through Claude Code', 'Not installed'],
  ['G', 'ChatGPT', 'Your ChatGPT Plus or Pro plan. Sign in; nothing to install', 'Not signed in'],
  ['X', 'Grok', 'Your SuperGrok or X Premium+ subscription', 'Not installed'],
  ['R', 'OpenRouter', 'One key for Claude, GPT, Gemini, Llama and more', 'Bring your own'],
  ['L', 'Local model', 'Ollama or LM Studio on this computer. Private and free.', 'Not running'],
]
const ROW = { y: 410, h: 72, gap: 8 }
const rowY = (i: number) => ROW.y + i * (ROW.h + ROW.gap)
const ACT = { y: 900 } // the window's bottom row: Test connection … Continue
const TEST = { x: 995, y: ACT.y + 28 }, NEXT = { x: 1668, y: ACT.y + 28 }

/** The first screen's top: the mark and the three steps of setup. */
function Onboard({ step, children }: { step: number; children: ReactNode }) {
  return (
    <Win style={{ height: '100%' }}>
      <div style={{ position: 'absolute', left: 40, top: 26, display: 'flex', alignItems: 'center', gap: 22 }}>
        <div style={{ width: 54, height: 54 }}><AppIcon size={54} /></div>
        <div style={{ display: 'flex', gap: 8 }}>{[1, 2, 3].map((i) => <i key={i} style={{ width: 34, height: 5, borderRadius: 3, background: i <= step ? C.text : 'rgba(255,255,255,0.14)' }} />)}</div>
      </div>
      {children}
    </Win>
  )
}
const h1: CSSProperties = { fontFamily: F.ui, fontSize: 40, fontWeight: 500, color: C.text, margin: 0, lineHeight: 1.15 }
const lead: CSSProperties = { fontSize: 22, color: C.text2, margin: '8px 0 0', lineHeight: 1.4 }
/** Placed at window coordinates (the film's x, y), inside the window. */
const at = (x: number, y: number, w?: number): CSSProperties => ({ position: 'absolute', left: x - W.x, top: y - W.y - 46, width: w })

export function Account() {
  const t = useBeat()
  const c = cues('account')
  const picked = t >= c.pick
  const signed = t >= c.plan
  const sheet = p(t, c.sheet, 0.8) * (1 - p(t, c.allow + 0.2, 0.6, ease.in))
  const tested = t >= c.ok
  return (
    <AbsoluteFill>
      <div style={side}><StepSide n={2} title={'Pick the AI you\nalready pay for.'} gl="No API key and no extra bill: your bots run on your plan." glAt={c.gloss} /></div>
      <Enter at={c.win} x={W.x} y={W.y} w={W.w} h={W.h}>
        <Onboard step={1}>
          <div style={at(IN.x, 300, 880)}>
            <p style={h1}>Which AI should your bots use?</p>
            <p style={lead}>Mimir runs on the AI subscription you already have. We never bill you for it.</p>
          </div>
          {ACCOUNTS.map(([mark, name, desc, state], i) => {
            const on = i === 1 && picked
            const live = i === 1 && signed
            return (
              <div key={name} style={{ ...at(IN.x, rowY(i), 880), height: ROW.h, display: 'flex', alignItems: 'center', gap: 18, padding: '0 20px', borderRadius: 14, background: on ? 'rgba(255,255,255,0.07)' : 'rgba(255,255,255,0.025)', boxShadow: `inset 0 0 0 ${on ? 2 : 1}px ${on ? 'rgba(236,233,225,0.5)' : C.line}` }}>
                <span style={{ width: 42, height: 42, flex: 'none', borderRadius: 11, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,0.08)', fontFamily: F.serif, fontSize: 24, color: C.text }}>{mark}</span>
                <span style={{ flex: 1, minWidth: 0 }}>
                  <b style={{ display: 'block', fontSize: 23, fontWeight: 600 }}>{name}</b>
                  <span style={{ display: 'block', fontSize: 18, color: C.text3, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{desc}</span>
                </span>
                {live ? <Badge live k={p(t, c.plan, 0.5, ease.back)}>Plus plan</Badge> : i === 3 ? <span style={{ fontSize: 19, color: C.text3 }}>{state}</span> : <Badge>{state}</Badge>}
              </div>
            )
          })}
          <div style={{ ...at(IN.x, 818, 880), height: 56, display: 'flex', alignItems: 'center', gap: 16, opacity: p(t, c.pick + 0.3, 0.6) }}>
            {signed
              ? <span style={{ fontSize: 21, color: C.text2, opacity: p(t, c.plan, 0.6) }}>You’re using your ChatGPT plan. Set how much Mimir may use each week in ChatGPT’s settings.</span>
              : <Btn icon="bolt" press={near(t, c.signin, 0.15)} hot={near(t, c.signin)}>Continue with ChatGPT</Btn>}
          </div>
          <div style={{ ...at(IN.x, ACT.y, 880), height: 56, display: 'flex', alignItems: 'center', gap: 18 }}>
            <Btn icon="bolt" press={near(t, c.test, 0.15)} hot={near(t, c.test)} style={{ opacity: signed ? 1 : 0.4 }}>{t >= c.test && !tested ? 'Testing…' : 'Test connection'}</Btn>
            {tested && <span style={{ fontSize: 21, color: C.live, opacity: p(t, c.ok, 0.5), display: 'flex', alignItems: 'center', gap: 8 }}><Icon name="check" size={20} />Works — replied in 1.4s</span>}
            <span style={{ flex: 1 }} />
            <Btn kind="primary" press={near(t, c.next, 0.15)} size={25} style={{ padding: '13px 30px', opacity: tested ? 1 : 0.35 }}>Continue</Btn>
          </div>
        </Onboard>
      </Enter>
      {/* signing in to ChatGPT, in the browser */}
      {sheet > 0 && <div style={{ position: 'absolute', left: W.x, top: W.y, width: W.w, height: W.h, borderRadius: 22, background: 'rgba(5,4,3,0.45)', opacity: sheet }} />}
      {sheet > 0 && (
        <div style={{ position: 'absolute', left: 1010, top: 330, width: 600, opacity: sheet, translate: `0 ${(1 - sheet) * 50}px`, scale: 0.96 + 0.04 * sheet }}>
          <Paper style={{ padding: '30px 36px 32px', textAlign: 'center' }}>
            <div style={{ fontFamily: F.mono, fontSize: 18, color: C.pInk3, letterSpacing: '0.06em' }}>auth.openai.com</div>
            <div style={{ fontFamily: F.serif, fontSize: 40, marginTop: 14 }}>Sign in to continue to Mimir</div>
            <div style={{ fontSize: 23, color: C.pInk2, marginTop: 12 }}>you@example.com · ChatGPT Plus</div>
            <div style={{ display: 'inline-block', marginTop: 26, fontSize: 24, padding: '13px 40px', borderRadius: 12, background: C.pInk, color: C.paper, scale: 1 - near(t, c.allow, 0.15) * 0.06 }}>Continue</div>
          </Paper>
        </div>
      )}
      <Pointer
        ways={[[1.5, 1560, 1040], [2.9, 1150, rowY(1) + 36], [3.5, 1150, rowY(1) + 36], [4.9, 1060, 846], [5.6, 1060, 846], [6.9, 1310, 625], [7.6, 1310, 625], [9.4, TEST.x, TEST.y], [10.4, TEST.x, TEST.y], [13.4, NEXT.x, NEXT.y], [14.6, NEXT.x + 30, NEXT.y + 40]]}
        clicks={[c.pick, c.signin, c.allow, c.test, c.next]} to={14.8} />
    </AbsoluteFill>
  )
}

// ---------------- 3. Your first bot ----------------

const LOOKS = [6, 4, 7, BOT.ada] // the face it starts with, then the three you tap through
const PICK = { x: 902, y: 455, gap: 92 }
const ROLE = "You're my personal assistant. Keep answers short."
const JOBS: [string, string][] = [['clock', 'Morning briefer'], ['check', 'Follow-up keeper'], ['search', 'Researcher'], ['globe', 'Page watcher'], ['history', 'Weekly reviewer']]

export function Name() {
  const t = useBeat()
  const c = cues('name')
  const look = LOOKS[[c.f1, c.f2, c.f3].filter((b) => t >= b).length]
  const picker = p(t, c.face + 0.3, 0.5) * (1 - p(t, c.f3 + 0.6, 0.5, ease.in))
  const typed = t >= c.type
  const role = t >= c.chip
  const ready = p(t, c.ready, 0.9)
  const nameText = typed ? <Typed text="Ada" at={c.type} dur={0.75} caret={t < c.chip} /> : 'Mimir'
  return (
    <AbsoluteFill>
      <div style={side}>
        <StepSide n={3} title={'Name your\nfirst bot.'} gl="Add more later, each with its own job:" glAt={c.gloss} />
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12, marginTop: 24, width: 600 }}>
          {JOBS.map(([icon, job], i) => <Tag key={job} icon={icon} k={p(t, c.jobs + i * 0.2, 0.6)}>{job}</Tag>)}
        </div>
      </div>
      <Enter at={-2} x={W.x} y={W.y} w={W.w} h={W.h}>
        <Onboard step={ready > 0.5 ? 3 : 2}>
          <div style={{ position: 'absolute', inset: 0, opacity: 1 - ready }}>
            <div style={{ ...at(IN.x, 300, 880), display: 'flex', gap: 26, alignItems: 'center' }}>
              <span style={{ position: 'relative', flex: 'none' }}>
                <Face id={look} size={96} moods={[[0, 'idle'], [c.type - 0.4, 'listening'], [c.f1, 'happy'], [c.f3 + 1.2, 'idle']]} />
                <span style={{ position: 'absolute', right: -4, bottom: 2, width: 32, height: 32, borderRadius: 16, background: C.surface, boxShadow: `inset 0 0 0 1px ${C.line}`, display: 'grid', placeItems: 'center', color: C.text2 }}><Icon name="pencil" size={17} /></span>
              </span>
              <div>
                <p style={h1}>Name your first bot</p>
                <p style={lead}>You can add more later, each with its own job. Tap it to change how it looks.</p>
              </div>
            </div>
            <div style={at(IN.x, 430, 880)}>
              <div style={{ fontSize: 21, fontWeight: 600, color: C.text2, marginBottom: 10 }}>Name</div>
              <Field focus={t > c.type - 0.6 && t < c.chip}>
                {typed ? nameText : <span style={{ background: t > c.type - 0.5 ? 'rgba(120,150,255,0.35)' : undefined, borderRadius: 3 }}>{nameText}</span>}
              </Field>
            </div>
            <div style={at(IN.x, 548, 880)}>
              <div style={{ fontSize: 21, fontWeight: 600, color: C.text2, marginBottom: 10 }}>What should it focus on? <span style={{ fontWeight: 400, color: C.text3 }}>Optional</span></div>
              <Field ph="Describe its job in a sentence or two." style={{ minHeight: 96, alignItems: 'flex-start' }}>{role ? ROLE : ''}</Field>
            </div>
            <div style={{ ...at(IN.x, 724, 880), display: 'flex', flexWrap: 'wrap', gap: 10 }}>
              {[ROLE, 'Help me stay on top of tasks and follow-ups.'].map((r, i) => (
                <span key={r} style={{ fontSize: 20, padding: '9px 16px', borderRadius: 99, color: i === 0 && role ? C.app : C.text2, background: i === 0 && role ? C.text : 'rgba(255,255,255,0.06)', boxShadow: `inset 0 0 0 1px ${C.line}`, scale: i === 0 ? 1 - near(t, c.chip, 0.15) * 0.05 : 1 }}>{r}</span>
              ))}
            </div>
            <div style={{ ...at(IN.x, ACT.y, 880), height: 56, display: 'flex', alignItems: 'center' }}>
              <Btn kind="ghost">Back</Btn><span style={{ flex: 1 }} />
              <Btn kind="primary" press={near(t, c.next, 0.15)} size={25} style={{ padding: '13px 30px' }}>Continue</Btn>
            </div>
            {picker > 0 && (
              <div style={{ position: 'absolute', left: PICK.x - W.x - 56, top: PICK.y - W.y - 46 - 56, opacity: picker, translate: `0 ${(1 - picker) * -16}px`, display: 'flex', gap: 92 - 64, padding: '24px 24px', borderRadius: 18, background: '#2A2A2E', boxShadow: `0 20px 40px rgba(0,0,0,0.6), inset 0 0 0 1px ${C.line}` }}>
                {[3, 4, 7, BOT.ada, 5].map((id) => (
                  <span key={id} style={{ borderRadius: 40, boxShadow: look === id ? `0 0 0 3px ${pigment(id)}` : undefined, padding: 4 }}><Face id={id} size={56} /></span>
                ))}
              </div>
            )}
          </div>
          {ready > 0 && (
            <div style={{ position: 'absolute', inset: 0, opacity: ready, translate: `0 ${(1 - ready) * 30}px` }}>
              <div style={{ ...at(IN.x, 300, 880), display: 'flex', gap: 26, alignItems: 'center' }}>
                <Face id={BOT.ada} size={96} moods={[[0, 'happy'], [c.ready + 1.4, 'idle']]} />
                <div>
                  <p style={h1}>Ada is ready</p>
                  <p style={lead}>Tell it about yourself. Everything it remembers shows up in Memory, where you can correct or forget it.</p>
                </div>
              </div>
              <div style={{ ...at(IN.x, 460, 880), display: 'flex', flexDirection: 'column', gap: 26 }}>
                {[['chat', '“Remember that I prefer short answers.”'], ['clock', '“Every weekday at 8:30, remind me of my open tasks.”'], ['telegram', 'Connect Telegram, Slack or Discord later from the bot’s page.']].map(([icon, s]) => (
                  <div key={s} style={{ display: 'flex', alignItems: 'center', gap: 18, fontSize: 24, color: C.text2 }}><Icon name={icon} size={26} />{s}</div>
                ))}
              </div>
              <div style={{ ...at(IN.x, ACT.y, 880), height: 56, display: 'flex', justifyContent: 'flex-end' }}>
                <Btn kind="primary" size={25} style={{ padding: '13px 30px' }}>Start chatting</Btn>
              </div>
            </div>
          )}
        </Onboard>
      </Enter>
      <Pointer
        ways={[[1.0, 1560, 1040], [1.8, 1180, 520], [2.6, 1180, 520], [3.8, 1120, 745], [4.5, 1120, 745], [5.8, 930, 360], [6.5, 930, 360],
          [6.95, PICK.x + PICK.gap + 10, PICK.y + 10], [7.45, PICK.x + 2 * PICK.gap + 10, PICK.y + 10], [7.95, PICK.x + 3 * PICK.gap + 10, PICK.y + 10], [8.5, PICK.x + 3 * PICK.gap + 10, PICK.y + 10],
          [9.4, NEXT.x, NEXT.y], [10.4, NEXT.x + 40, NEXT.y + 60]]}
        clicks={[c.chip, c.face, c.f1, c.f2, c.f3, c.next]} to={10.6} />
    </AbsoluteFill>
  )
}

// ---------------- 3, again: tell it about you ----------------

const TABS = ['Chats', 'Needs you', 'Memory', 'Routines']
const TAB = { x: IN.x, y: W.y + 46 + 22, w: 170, gap: 8 }
const MEM = { x: TAB.x + 2 * (TAB.w + TAB.gap) + TAB.w / 2, y: TAB.y + 26 }

export function Hello() {
  const t = useBeat()
  const c = cues('hello')
  const fly = p(t, c.fly, c.land - c.fly, ease.io)
  const landed = t >= c.land
  const drop = p(t, c.land + 0.4, 0.8)
  const chip = { x: 1180, y: 836 }
  return (
    <AbsoluteFill>
      <div style={side}><StepSide n={3} title={'Then tell it\nabout you.'} gl="Everything it remembers shows up in Memory, where you can correct or forget it." glAt={c.gloss} /></div>
      <Enter at={-1} x={W.x} y={W.y} w={W.w} h={W.h}>
        <Win style={{ height: '100%' }}>
          <div style={{ position: 'absolute', left: 40, top: 22, display: 'flex', gap: TAB.gap }}>
            {TABS.map((tab, i) => (
              <span key={tab} style={{ width: TAB.w, height: 52, borderRadius: 12, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10, fontSize: 21, color: i === 0 ? C.text : C.text2, background: i === 0 ? 'rgba(255,255,255,0.08)' : undefined, position: 'relative',
                scale: i === 2 && landed ? 1 + 0.08 * Math.sin(p(t, c.land, 0.6) * Math.PI) : 1 }}>
                {tab}
                {i === 2 && landed && <span style={{ minWidth: 26, height: 26, borderRadius: 13, background: C.text, color: C.app, fontSize: 16, fontWeight: 700, display: 'grid', placeItems: 'center' }}>1</span>}
              </span>
            ))}
          </div>
          <div style={{ position: 'absolute', left: 40, right: 40, top: 96, display: 'flex', alignItems: 'center', gap: 16, paddingBottom: 18, borderBottom: `1px solid ${C.line}` }}>
            <Face id={BOT.ada} size={50} />
            <div><div style={{ fontFamily: F.ui, fontSize: 27, fontWeight: 500 }}>Ada</div><div style={{ fontSize: 18, color: C.text3 }}>Built-in chat</div></div>
          </div>
          <div style={{ position: 'absolute', left: 40, right: 40, top: 190, bottom: 110, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', overflow: 'hidden', maskImage: 'linear-gradient(transparent, #000 50px)' }}>
            <Bot at={-4} moods={[[0, 'idle']]}><span>Hi! I’m Ada. Tell me a bit about yourself, and I’ll remember it.</span></Bot>
            <You at={c.send}>Remember that my dentist is Dr. Lee in Stellenbosch.</You>
            <Bot at={c.reply} text="Saved: Dr. Lee in Stellenbosch. Want a reminder before your next check-up?" dur={1.4} wait="Remembering"
              moods={[[0, 'idle'], [c.send + 0.2, 'remembering'], [c.reply, 'writing'], [c.chip, 'happy'], [c.chip + 1.5, 'idle']]}
              below={<Grow k={p(t, c.chip, 0.5)}><div style={{ paddingTop: 14 }}><Tag icon="memory" k={p(t, c.chip, 0.6)} style={{ opacity: fly > 0 ? 0.5 : 1 }}>Saved to memory · Dentist</Tag></div></Grow>} />
          </div>
          <div style={{ position: 'absolute', left: 34, right: 34, bottom: 26 }}>
            <Composer text="Remember that my dentist is Dr. Lee in Stellenbosch." at={c.type} dur={2} send={c.send} />
          </div>
        </Win>
      </Enter>
      {/* the memory slips up into the Memory tab */}
      {fly > 0 && fly < 1 && (
        <div style={{ position: 'absolute', left: lerp(chip.x, MEM.x, fly), top: lerp(chip.y, MEM.y, fly) - Math.sin(fly * Math.PI) * 60, translate: '-50% -50%', rotate: `${(1 - fly) * -8}deg`, scale: lerp(1, 0.35, fly) }}>
          <Paper style={{ width: 300, padding: '14px 18px' }}><div style={{ fontFamily: F.serif, fontSize: 26 }}>Dentist</div><div style={{ fontSize: 18, color: C.pInk2 }}>Dr. Lee in Stellenbosch</div></Paper>
        </div>
      )}
      {drop > 0 && (
        <div style={{ position: 'absolute', left: MEM.x - 90, top: MEM.y + 44, opacity: drop * (1 - p(t, 11.2, 0.5, ease.in)), translate: `0 ${(1 - drop) * -20}px` }}>
          <Paper style={{ width: 560, padding: '22px 26px 18px' }}>
            <div style={{ fontFamily: F.serif, fontSize: 36, lineHeight: 1.05 }}>Dentist</div>
            <div style={{ fontSize: 24, lineHeight: 1.4, color: C.pInk2, marginTop: 8 }}>Dr. Lee in Stellenbosch.</div>
            <div style={{ fontFamily: F.mono, fontSize: 16, color: C.pInk3, display: 'flex', gap: 18, marginTop: 12 }}><span>you said</span><span>just now</span><span>shared by all your bots</span></div>
          </Paper>
        </div>
      )}
    </AbsoluteFill>
  )
}

// ---------------- 4. Your phone ----------------

const PANEL = { x: 680, y: 210, w: 720, h: 590 }
const PIN = { x: PANEL.x + 36, y: PANEL.y + 46 } // inside the panel
const pin = (y: number, w = PANEL.w - 72): CSSProperties => ({ position: 'absolute', left: 36, top: y, width: w })
const TOKEN = '7342918805:AAH3k-Qz9vLx2mPfT'

export function Pocket() {
  const t = useBeat()
  const c = cues('pocket')
  const ok = t >= c.ok
  const step = (n: number, text: ReactNode, k = 1) => (
    <div style={{ display: 'flex', gap: 16, fontSize: 23, lineHeight: 1.4, color: C.text2, opacity: k }}>
      <span style={{ fontFamily: F.serif, fontSize: 28, color: C.gall, width: 20, lineHeight: 1.15 }}>{n}</span><span>{text}</span>
    </div>
  )
  const code = (s: string) => <span style={{ fontFamily: F.mono, fontSize: 21, color: C.text, background: 'rgba(255,255,255,0.07)', padding: '2px 8px', borderRadius: 6 }}>{s}</span>
  return (
    <AbsoluteFill>
      <div style={{ ...side, top: 230 }}><StepSide n={4} title={'Put it on\nyour phone.'} width={500} size={84} gl="Slack and Discord work the same way. Nothing to open up: it connects out." glAt={c.gloss} /></div>
      <Enter at={c.panel} x={PANEL.x} y={PANEL.y} w={PANEL.w} h={PANEL.h}>
        <Win title="Ada · Settings" style={{ height: '100%' }}>
          <div style={{ ...pin(28), display: 'flex', alignItems: 'center', gap: 14 }}>
            <Icon name="telegram" size={30} />
            <span style={{ fontFamily: F.ui, fontSize: 30, fontWeight: 500, flex: 1 }}>Telegram</span>
            {ok ? <Badge live k={p(t, c.ok, 0.5, ease.back)}>@ada_home_bot</Badge> : <Badge>Not connected</Badge>}
          </div>
          <div style={pin(100)}>{step(1, <>In Telegram, message {code('@BotFather')} and send {code('/newbot')}.</>)}</div>
          <div style={pin(162)}>{step(2, 'Paste the token it gives you:')}</div>
          <div style={{ ...pin(210), display: 'flex', gap: 14 }}>
            <Field mono focus={t > c.token - 0.4 && t < c.connect} style={{ flex: 1, fontSize: 21 }} ph="123456:ABC…">{t >= c.token ? <Typed text={TOKEN} at={c.token} dur={1.25} caret={t < c.connect} /> : ''}</Field>
            <Btn kind={ok ? 'ok' : 'primary'} icon={ok ? 'check' : undefined} press={near(t, c.connect, 0.15)}>{ok ? 'Connected' : 'Connect'}</Btn>
          </div>
          <div style={pin(310)}>{step(3, <>Pair your chat: open the link, then press Start.</>, p(t, c.pair, 0.7))}</div>
          <div style={{ ...pin(362), opacity: p(t, c.pair + 0.3, 0.7) }}>
            <div style={{ marginLeft: 36, display: 'inline-flex', fontFamily: F.mono, fontSize: 22, padding: '12px 18px', borderRadius: 12, background: 'rgba(255,255,255,0.06)', boxShadow: `inset 0 0 0 1px ${C.line}`, color: C.text }}>t.me/ada_home_bot?start=K7Q2PX</div>
          </div>
          <div style={{ ...pin(460), display: 'flex', alignItems: 'center', gap: 14, opacity: p(t, c.paired, 0.6), translate: `0 ${(1 - p(t, c.paired, 0.6)) * 14}px` }}>
            <span style={{ width: 14, height: 14, borderRadius: 7, background: C.live }} />
            <span style={{ fontSize: 24 }}>Paired with your chat</span>
            <span style={{ fontSize: 20, color: C.text3 }}>· only you can talk to it</span>
          </div>
        </Win>
      </Enter>
      <div style={{ position: 'absolute', left: 1440, top: 120, transformOrigin: '0 0', scale: 0.92 }}>
        <Phone k={p(t, c.phone, 1.2)} clock="9:41" moods={[[0, 'idle'], [c.start + 0.3, 'happy'], [c.start + 2, 'idle']]}>
          {t >= c.phone && <div style={{ textAlign: 'center', fontSize: 17, color: C.text3, paddingBottom: 6 }}>Today</div>}
          <Bubble me at={c.start}>/start K7Q2PX</Bubble>
          <Bubble at={c.paired}>Paired! Hi, I’m Ada. Tell me about yourself, or ask me anything.</Bubble>
        </Phone>
      </div>
      <Pointer ways={[[0.9, 1250, 1010], [1.3, PIN.x + 320, PANEL.y + 46 + 238], [2.9, PIN.x + 320, PANEL.y + 46 + 238], [3.4, PIN.x + 560, PANEL.y + 46 + 238], [4.3, PIN.x + 560, PANEL.y + 46 + 238], [5.2, 1300, 960]]} clicks={[c.connect]} to={5.4} />
    </AbsoluteFill>
  )
}

// ---------------- 5. Email, calendar, search ----------------

const PLUGS: { icon: string; title: string; desc: string; label: string; value: string; cue: [string, string]; badge: string; mono?: boolean }[] = [
  { icon: 'inbox', title: 'Email', desc: 'Gmail, iCloud, Fastmail or any IMAP. An app password, not your password.', label: 'App password', value: '•••• •••• •••• ••••', cue: ['mail', 'mailok'], badge: '41 unread' },
  { icon: 'clock', title: 'Calendar', desc: 'Your calendar’s private iCal link. Bots read it, and add events when you say so.', label: 'Calendar link', value: 'https://calendar.google.com/…/basic.ics', cue: ['cal', 'calok'], badge: '14 events this week', mono: true },
  { icon: 'globe', title: 'Web search', desc: 'A free Brave Search key, for AI accounts other than Claude.', label: 'Brave Search key', value: 'BSA•••••••••••••••4f2', cue: ['web', 'webok'], badge: 'Working', mono: true },
]

export function Plug() {
  const t = useBeat()
  const c = cues('plug')
  return (
    <AbsoluteFill>
      <div style={side}><StepSide n={5} title={'Connect email,\ncalendar and\nsearch.'} size={84} gl="An app password, a calendar link, a free search key. A few minutes each." glAt={c.gloss} /></div>
      <Enter at={c.panel} x={W.x} y={W.y} w={W.w} h={W.h}>
        <Win title="Settings" style={{ height: '100%' }}>
          <div style={{ position: 'absolute', left: 40, right: 40, top: 26 }}>
            <Field ph="Search settings" style={{ gap: 12 }} />
          </div>
          {PLUGS.map((g, i) => {
            const [go, done] = g.cue
            const on = t >= c[done]
            const typing = t >= c[go]
            return (
              <div key={g.title} style={{ position: 'absolute', left: 40, right: 40, top: 120 + i * 210, padding: '22px 26px', borderRadius: 16, background: 'rgba(255,255,255,0.03)', boxShadow: `inset 0 0 0 1px ${on ? 'rgba(76,191,130,0.35)' : C.line}` }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                  <Icon name={g.icon} size={28} />
                  <span style={{ fontFamily: F.ui, fontSize: 30, fontWeight: 500, flex: 1 }}>{g.title}</span>
                  {on ? <Badge live k={p(t, c[done], 0.5, ease.back)}>{g.badge}</Badge> : <Badge>Not set up</Badge>}
                </div>
                <div style={{ fontSize: 20, color: C.text3, marginTop: 6 }}>{g.desc}</div>
                <Field mono={g.mono} focus={typing && !on} ph={g.label} style={{ marginTop: 14, fontSize: g.mono ? 20 : 24 }}>
                  {typing ? (i === 0 ? <Typed text={g.value} at={c[go]} dur={0.75} caret={!on} /> : <span style={{ opacity: p(t, c[go], 0.3) }}>{g.value}</span>) : ''}
                </Field>
              </div>
            )
          })}
        </Win>
      </Enter>
    </AbsoluteFill>
  )
}

// ---------------- 6. Skills ----------------

const LIB: [string, string, string][] = [
  ['clock', 'Morning briefing', 'What’s due and what matters today, first thing'],
  ['inbox', 'Inbox triage', 'What needs a reply, and what can wait'],
  ['user', 'Meeting prep', 'Who’s coming, and what you last discussed'],
  ['history', 'Weekly review', 'What got done, what’s still open'],
  ['bolt', 'Price watch', 'Tells you when a price drops'],
  ['globe', 'Trip planner', 'Flights, stays and a plan for each day'],
]
const LROW = { y: W.y + 46 + 150, h: 92 }
const ly = (i: number) => LROW.y + i * LROW.h + LROW.h / 2
const ADD_X = 1690, SCHED_X = 1520

export function Library() {
  const t = useBeat()
  const c = cues('library')
  const pop = p(t, c.sched + 0.2, 0.5) * (1 - p(t, c.when + 0.2, 0.4, ease.in))
  return (
    <AbsoluteFill>
      <div style={side}><StepSide n={6} title={'Add a few\nskills.'} gl="Ready-made, in Mimir’s skill library. Schedule the daily ones." glAt={c.gloss} /></div>
      <Enter at={c.panel} x={W.x} y={W.y} w={W.w} h={W.h}>
        <Win title="Skills" style={{ height: '100%' }}>
          <div style={{ position: 'absolute', left: 40, right: 40, top: 30 }}>
            <p style={{ ...h1, fontSize: 36 }}>Mimir’s skill library</p>
            <p style={lead}>Step-by-step procedures, written for Mimir’s tools. Add the ones you want.</p>
          </div>
          {LIB.map(([icon, name, desc], i) => {
            const added = (i === 0 && t >= c.add) || (i === 1 && t >= c.add2)
            const set = i === 0 && t >= c.set
            const addAt = i === 0 ? c.add : c.add2
            return (
              <div key={name} style={{ position: 'absolute', left: 40, right: 40, top: LROW.y - W.y - 46 + i * LROW.h, height: LROW.h, display: 'flex', alignItems: 'center', gap: 18, borderTop: i ? `1px solid ${C.line}` : undefined }}>
                <span style={{ width: 54, height: 54, flex: 'none', borderRadius: 14, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,0.06)', color: C.text2 }}><Icon name={icon} size={28} /></span>
                <span style={{ flex: 1, minWidth: 0 }}>
                  <b style={{ display: 'block', fontSize: 24, fontWeight: 600 }}>{name}</b>
                  {set
                    ? <span style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 19, color: C.live, opacity: p(t, c.set, 0.5) }}><Icon name="clock" size={18} />Scheduled · weekdays at 7:30 · Ada</span>
                    : <span style={{ display: 'block', fontSize: 19, color: C.text3 }}>{desc}</span>}
                </span>
                {i === 0 && added && !set && <Btn kind="ghost" icon="clock" hot={near(t, c.sched)} press={near(t, c.sched, 0.15)} size={21} style={{ opacity: p(t, c.add + 0.2, 0.5) }}>Schedule it</Btn>}
                {added
                  ? <Btn kind="ok" icon="check" size={21} style={{ scale: 0.9 + 0.1 * p(t, addAt, 0.5, ease.back) }}>Added</Btn>
                  : <Btn icon="plus" size={21} press={(i === 0 && near(t, c.add, 0.15)) || (i === 1 && near(t, c.add2, 0.15)) ? 1 : 0}>Add</Btn>}
              </div>
            )
          })}
          {pop > 0 && (
            <div style={{ position: 'absolute', left: 1290 - W.x, top: ly(0) + 34 - W.y - 46, width: 420, padding: 8, borderRadius: 14, background: '#2A2A2E', boxShadow: `0 20px 40px rgba(0,0,0,0.6), inset 0 0 0 1px ${C.line}`, opacity: pop, translate: `0 ${(1 - pop) * -10}px`, zIndex: 3 }}>
              {['Every day at 7:30', 'Weekdays at 7:30', 'Custom…'].map((o, i) => (
                <div key={o} style={{ padding: '12px 16px', borderRadius: 9, fontSize: 22, background: i === 1 && t > c.when - 0.5 ? 'rgba(255,255,255,0.1)' : undefined }}>{o}</div>
              ))}
            </div>
          )}
        </Win>
      </Enter>
      <Pointer
        ways={[[1.0, 1500, 1040], [2.3, ADD_X, ly(0)], [2.9, ADD_X, ly(0)], [3.8, SCHED_X, ly(0)], [4.4, SCHED_X, ly(0)], [5.3, 1440, ly(0) + 34 + 8 + 48 + 24], [5.9, 1440, ly(0) + 34 + 8 + 48 + 24], [7.3, ADD_X, ly(1)], [8.0, ADD_X, ly(1)], [8.8, 1560, 1010]]}
        clicks={[c.add, c.sched, c.when, c.add2]} to={9} />
    </AbsoluteFill>
  )
}

