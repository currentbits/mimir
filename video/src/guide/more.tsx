// The rest of every day: notes from calls, a pause, what needs you, your own server; then the six steps again.
import type { ReactNode } from 'react'
import { AbsoluteFill, random } from 'remotion'
import { BOT, C, cues, ease, F, Face, Grow, headline, Icon, lerp, p, Panel, Paper, Ripple, Rise, Typed, useBeat, Words } from '../lib'
import { Lockup, Mark } from '../scenes/open'
import { Badge, Btn, Enter, Gloss, Head, near, Pointer, Win } from './kit'

// ---------------- Notes: transcribe a call, then a bot writes it up ----------------

const LINES: [string, string, string, string][] = [['l1', 'Thandi', '09:02', 'Let’s ship on Thursday.'], ['l2', 'Sam', '09:03', 'I’ll update the docs before then.'], ['l3', 'You', '09:03', 'Great. Ada, write it up.']]
const MW = { x: 260, y: 250, w: 900, h: 620 }
const START = { x: MW.x + MW.w / 2, y: MW.y + 46 + 328 }
const SUM = { x: MW.x + MW.w - 36 - 150, y: MW.y + MW.h - 30 - 27 }

export function Notes() {
  const t = useBeat()
  const c = cues('notes')
  const rec = t >= c.start
  const notes = p(t, c.notes, 1)
  const secs = 724 + Math.floor((t * 60) / 100)
  return (
    <AbsoluteFill>
      <Head title="Take notes in calls." at={c.head} />
      <Enter at={-1} x={MW.x} y={MW.y} w={MW.w} h={MW.h}>
        <Win title="Meetings" style={{ height: '100%' }}>
          {!rec ? (
            <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', paddingTop: 150, color: C.text2 }}>
              <Icon name="mic" size={56} />
              <div style={{ fontSize: 26, marginTop: 22 }}>Start a Teams, Zoom or Meet call, then:</div>
              <Btn kind="primary" icon="mic" size={26} press={near(t, c.start, 0.15)} style={{ marginTop: 30, padding: '14px 28px' }}>Start transcribing</Btn>
            </div>
          ) : (
            <div style={{ position: 'absolute', inset: '26px 36px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontSize: 24 }}>
                <span style={{ width: 14, height: 14, borderRadius: 7, background: C.danger, opacity: 0.55 + 0.45 * Math.cos(t * Math.PI) ** 2 }} />
                <span style={{ fontWeight: 600 }}>Recording</span><span style={{ color: C.text3 }}>· Standup</span>
                <span style={{ marginLeft: 'auto', fontFamily: F.mono, color: C.text3 }}>{Math.floor(secs / 60)}:{String(secs % 60).padStart(2, '0')}</span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 5, height: 90, marginTop: 18 }}>
                {Array.from({ length: 70 }, (_, i) => {
                  const n = Math.floor(t * 8) + i
                  const amp = 0.18 + 0.82 * random(`gw${n}`) * (0.55 + 0.45 * Math.sin(n * 0.37))
                  return <span key={i} style={{ flex: 1, height: `${Math.max(6, amp * 100)}%`, borderRadius: 3, background: i > 56 ? C.text : C.text3, opacity: i > 56 ? 0.9 : 0.4 + (i / 70) * 0.4 }} />
                })}
              </div>
              <div style={{ marginTop: 12 }}>
                {LINES.map(([cue, who, when, text]) => (
                  <Grow key={cue} k={p(t, c[cue], 0.6)}>
                    <div style={{ display: 'flex', gap: 20, padding: '13px 0', borderTop: `1px solid ${C.line}`, opacity: p(t, c[cue], 0.8) }}>
                      <span style={{ width: 110, flex: 'none', fontSize: 21, color: C.text2, fontWeight: 600 }}>{who}<span style={{ display: 'block', fontFamily: F.mono, fontSize: 16, color: C.text3, fontWeight: 400, marginTop: 3 }}>{when}</span></span>
                      <span style={{ fontSize: 27, lineHeight: 1.35 }}>{text}</span>
                    </div>
                  </Grow>
                ))}
              </div>
            </div>
          )}
          {t >= c.l3 && (
            <div style={{ position: 'absolute', right: 36, bottom: 30, opacity: p(t, c.l3 + 0.4, 0.5) }}>
              <Btn icon="wand" press={near(t, c.sum, 0.15)} hot={near(t, c.sum)}>Summarise with Ada</Btn>
            </div>
          )}
        </Win>
      </Enter>
      {notes > 0 && (
        <div style={{ position: 'absolute', left: 1190, top: 430, opacity: notes, translate: `${(1 - notes) * 40}px ${(1 - notes) * 60}px`, rotate: `${-1.8 + (1 - notes) * 4}deg` }}>
          <Paper style={{ width: 580, padding: '26px 30px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
              <Face id={BOT.ada} size={56} moods={[[0, 'writing'], [c.notes + 1.6, 'happy']]} />
              <div style={{ fontFamily: F.serif, fontSize: 38 }}>Notes · Standup</div>
            </div>
            <div style={{ fontSize: 26, lineHeight: 1.5, marginTop: 14 }}>
              <div><b style={{ fontWeight: 600 }}>Decided:</b> ship on Thursday.</div>
              <div><b style={{ fontWeight: 600 }}>To do:</b> Sam updates the docs.</div>
            </div>
          </Paper>
        </div>
      )}
      <Pointer ways={[[0.2, 900, 1000], [c.start - 0.2, START.x + 30, START.y + 8], [c.start + 0.6, START.x + 30, START.y + 8], [c.sum - 0.3, SUM.x, SUM.y + 4], [c.sum + 0.6, SUM.x, SUM.y + 4], [c.sum + 1.3, 1100, 1000]]} clicks={[c.start, c.sum]} to={c.sum + 1.4} />
      <Gloss at={c.gloss}>Teams, Zoom, Meet and phone calls, on your computer. No bot joins.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Hush: pause all of them until tomorrow ----------------

const SB = { x: 520, y: 250, w: 560, h: 600 }
const PAUSE = { x: SB.x + SB.w - 44, y: SB.y + 46 + 136 }
const CHOICES = ['Until I resume', 'For an hour', 'Until tomorrow 8:00', 'Until Monday 8:00']
const MENU = { x: PAUSE.x + 30, y: PAUSE.y + 10 }
const CREW: [number, string, string][] = [[BOT.ada, 'Ada', 'Telegram'], [BOT.researcher, 'Researcher', 'On the web'], [BOT.briefer, 'Morning briefer', '7:30 weekdays']]

export function Hush() {
  const t = useBeat()
  const c = cues('hush')
  const menu = p(t, c.menu, 0.5) * (1 - p(t, c.pick + 0.15, 0.35, ease.in))
  const asleep = t >= c.sleep
  const night = p(t, c.sleep, 1.5, ease.soft)
  const hover = t < c.pick - 1 ? -1 : t < c.pick - 0.35 ? 1 : 2
  const box = (icon: string) => <span style={{ width: 46, height: 46, flex: 'none', borderRadius: 12, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,0.06)', color: C.text2 }}><Icon name={icon} size={24} /></span>
  const row = (icon: ReactNode, name: string, sub: string, dim = false) => (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16, padding: '11px 14px', borderRadius: 12, opacity: dim ? 0.75 : 1 }}>
      {icon}
      <div><div style={{ fontSize: 24 }}>{name}</div><div style={{ fontSize: 18, color: C.text3, marginTop: 1 }}>{sub}</div></div>
    </div>
  )
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{ background: 'rgba(4,4,12,0.5)', opacity: night }} />
      <Head title="Need some quiet? Pause them." at={c.head} />
      <Enter at={-1} x={SB.x} y={SB.y} w={SB.w} h={SB.h}>
        <Win style={{ height: '100%', background: '#1D1D20' }}>
          <div style={{ position: 'absolute', inset: '20px 18px' }}>
            {row(box('inbox'), 'Needs you', 'All clear')}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '24px 14px 8px', fontSize: 19, color: C.text3, fontWeight: 500 }}>
              <span>Bots</span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', borderRadius: 9, color: asleep ? C.text : C.text2, background: near(t, c.pause) || menu > 0 ? 'rgba(255,255,255,0.1)' : undefined, scale: 1 - near(t, c.pause, 0.15) * 0.08 }}>
                <Icon name={asleep ? 'play' : 'pause'} size={20} />{asleep && <span style={{ fontSize: 18 }}>Resume all</span>}
              </span>
            </div>
            {CREW.map(([id, name, sub]) => row(<Face id={id} size={46} moods={[[0, 'idle'], [c.sleep + random(`z${id}`) * 0.4, 'asleep']]} />, name, asleep ? 'Paused · back tomorrow 8:00' : sub, asleep))}
            <div style={{ padding: '24px 14px 8px', fontSize: 19, color: C.text3, fontWeight: 500 }}>Chats</div>
            {row(box('laptop'), 'This computer', 'Built-in chat')}
          </div>
        </Win>
      </Enter>
      {menu > 0 && (
        <div style={{ position: 'absolute', left: MENU.x, top: MENU.y, opacity: menu, translate: `0 ${(1 - menu) * -10}px` }}>
          <Panel style={{ width: 400, padding: 8, borderRadius: 16, background: '#2A2A2E' }}>
            <div style={{ fontSize: 18, color: C.text3, padding: '10px 14px 6px' }}>Pause all bots</div>
            {CHOICES.map((o, i) => (
              <div key={o} style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 14px', borderRadius: 9, fontSize: 23, background: i === hover ? 'rgba(255,255,255,0.1)' : undefined }}>
                <Icon name={i ? 'clock' : 'pause'} size={20} />{o}
              </div>
            ))}
          </Panel>
        </div>
      )}
      {asleep && (
        <div style={{ position: 'absolute', left: 1220, top: 470, display: 'flex', alignItems: 'center', gap: 24, color: C.foam, opacity: p(t, c.sleep + 0.4, 1) }}>
          <Icon name="moon" size={70} />
          <div>
            <div style={{ fontFamily: F.serif, fontSize: 52, lineHeight: 1.05 }}>Back tomorrow</div>
            <div style={{ fontFamily: F.mono, fontSize: 22, color: C.ink3, marginTop: 8, letterSpacing: '0.08em' }}>AT 8:00 · NOTHING RUNS TILL THEN</div>
          </div>
        </div>
      )}
      <Pointer ways={[[0.2, 1300, 1000], [c.pause - 0.2, PAUSE.x, PAUSE.y + 6], [c.pause + 0.6, PAUSE.x, PAUSE.y + 6], [c.pick - 1, MENU.x + 180, MENU.y + 122], [c.pick - 0.6, MENU.x + 180, MENU.y + 122], [c.pick - 0.15, MENU.x + 180, MENU.y + 174], [c.pick + 0.5, MENU.x + 180, MENU.y + 174], [c.pick + 1.3, 1500, 1000]]} clicks={[c.pause, c.pick]} to={c.pick + 1.4} />
      <Gloss at={c.gloss}>All of them, or one. Nothing runs on its own until they’re back.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Inbox: Needs you, and the menu bar ----------------

const NW = { x: 360, y: 250, w: 1200, h: 600 }
const NROW = 150
const SURE = { x: NW.x + NW.w - 36 - 110, y: NW.y + 46 + 24 + 2 * NROW + NROW / 2 }

export function Inbox() {
  const t = useBeat()
  const c = cues('inbox')
  const count = t < c.badge ? 0 : t < c.gone ? 3 : 2
  const item = (cue: string, icon: ReactNode, title: string, sub: string, actions: ReactNode, k = 1) => (
    <Grow k={k}>
      <div style={{ height: NROW, display: 'flex', alignItems: 'center', gap: 24, padding: '0 10px', borderBottom: `1px solid ${C.line}`, opacity: p(t, c[cue], 0.6), translate: `${(1 - p(t, c[cue], 0.6)) * 30}px 0` }}>
        {icon}
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 28, fontWeight: 600 }}>{title}</div>
          <div style={{ fontSize: 22, color: C.text2, marginTop: 4 }}>{sub}</div>
        </div>
        <div style={{ display: 'flex', gap: 10 }}>{actions}</div>
      </div>
    </Grow>
  )
  return (
    <AbsoluteFill>
      {/* the menu bar, with Mimir's icon and what's waiting */}
      <div style={{ position: 'absolute', left: 0, right: 0, top: 0, height: 52, background: 'rgba(30,26,22,0.8)', boxShadow: '0 1px 0 rgba(236,233,225,0.08)', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 34, padding: '0 40px', fontFamily: F.sans, fontSize: 23, color: C.foam, opacity: p(t, 0, 0.8) }}>
        <span style={{ position: 'relative', display: 'flex', scale: count && t < c.badge + 0.4 ? 1 + 0.2 * Math.sin(p(t, c.badge, 0.4) * Math.PI) : 1 }}>
          <Mark id="mb" size={36} />
          {count > 0 && <span style={{ position: 'absolute', right: -16, top: -6, minWidth: 26, height: 26, borderRadius: 13, background: C.danger, color: '#fff', fontSize: 16, fontWeight: 700, display: 'grid', placeItems: 'center' }}>{count}</span>}
        </span>
        <span style={{ opacity: 0.6 }}>Tue 8:02</span>
      </div>
      <Head title="One place for what needs you." at={c.head} />
      <Enter at={-1} x={NW.x} y={NW.y} w={NW.w} h={NW.h}>
        <Win title="Needs you" style={{ height: '100%' }}>
          <div style={{ position: 'absolute', left: 36, right: 36, top: 24 }}>
            {item('i1', <Face id={BOT.ada} size={64} moods={[[0, 'asking']]} />, 'Ada wants to send an email to Sam', '“Padel moved to 18:30. See you there!”', <><Btn kind="ghost">Deny</Btn><Btn kind="primary">Allow</Btn></>)}
            {item('i2', <Face id={BOT.briefer} size={64} moods={[[0, 'sad']]} />, 'Morning briefer didn’t run at 7:30', 'This computer was asleep.', <Btn icon="play">Run now</Btn>)}
            {item('i3', <span style={{ width: 64, height: 64, borderRadius: 16, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,0.06)', color: C.text2 }}><Icon name="memory" size={32} /></span>,
              '2 memories to confirm', 'Learned while Ada read a web page', <Btn press={near(t, c.ok, 0.15)} hot={near(t, c.ok)}>All look right</Btn>, 1 - p(t, c.gone, 0.6, ease.io))}
          </div>
        </Win>
      </Enter>
      <Pointer ways={[[1.5, 1500, 1010], [c.ok - 0.2, SURE.x, SURE.y + 6], [c.ok + 0.6, SURE.x, SURE.y + 6], [c.ok + 1.3, 1560, 1000]]} clicks={[c.ok]} to={c.ok + 1.4} />
      <Gloss at={c.gloss}>Approvals, missed routines, memories to confirm. Also in your menu bar.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Always: on your own server ----------------

const BOX = { x: 1330, y: 560 }

export function Always() {
  const t = useBeat()
  const c = cues('always')
  const box = p(t, c.box, 1, ease.back)
  const hops = [c.hop1, c.hop2, c.hop3]
  const app = p(t, c.app, 0.9)
  return (
    <AbsoluteFill>
      <Head title="On all night? Use your server." at={c.head} />
      <div style={{ position: 'absolute', left: 170, top: 420, width: 760, opacity: p(t, c.term - 0.6, 0.6) }}>
        <div style={{ fontFamily: F.mono, fontSize: 34, color: C.foam, padding: '26px 34px', borderRadius: 18, background: 'rgba(0,0,0,0.5)', boxShadow: 'inset 0 0 0 1px rgba(236,233,225,0.12)' }}>
          <div><span style={{ color: C.ink3 }}>$ </span><Typed text="docker compose up -d" at={c.term} dur={1} caret={t < c.box} /></div>
          <Grow k={p(t, c.term + 1.3, 0.5)}><div style={{ fontSize: 26, color: C.live, paddingTop: 16 }}>✔ Container mimir  Started</div></Grow>
        </div>
        <Rise at={c.term + 0.3}><div style={{ fontFamily: F.mono, fontSize: 20, color: C.ink3, marginTop: 18, letterSpacing: '0.04em' }}>a VPS · a home server · a Raspberry Pi</div></Rise>
      </div>
      <div style={{ position: 'absolute', left: BOX.x - 190, top: BOX.y - 100, width: 380, opacity: Math.min(1, box * 1.5), scale: 0.85 + 0.15 * box }}>
        <div style={{ height: 200, borderRadius: 22, background: 'linear-gradient(#3A3229, #221D17)', boxShadow: '0 30px 60px -20px rgba(0,0,0,0.85), inset 0 1.5px 0 rgba(255,240,220,0.12)', padding: '26px 30px', display: 'flex', flexDirection: 'column', justifyContent: 'space-between' }}>
          {[0, 1, 2].map((i) => <div key={i} style={{ height: 6, borderRadius: 3, background: 'rgba(0,0,0,0.35)' }} />)}
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            {[0, 1, 2].map((i) => <span key={i} style={{ width: 11, height: 11, borderRadius: 6, background: C.live, opacity: 0.45 + 0.55 * Math.abs(Math.sin(t * 2.1 + i * 1.7)) }} />)}
            <span style={{ marginLeft: 'auto', fontFamily: F.serif, fontSize: 30, color: C.foam, opacity: 0.85 }}>ᛗ</span>
          </div>
        </div>
      </div>
      {[BOT.ada, BOT.researcher, BOT.watcher].map((id, i) => {
        const k = p(t, hops[i], 0.8, ease.io)
        const shown = p(t, c.term - 0.6 + i * 0.15, 0.6, ease.back)
        const x0 = 330 + i * 96, y0 = 420 - 37, x1 = BOX.x + (i - 1) * 96, y1 = BOX.y - 137
        return (
          <div key={id} style={{ position: 'absolute', left: lerp(x0, x1, k) - 41, top: lerp(y0, y1, k) - Math.sin(k * Math.PI) * 200 - 41, scale: shown, zIndex: 3 }}>
            <Face id={id} size={82} moods={[[0, 'idle'], [hops[i] - 0.2, 'happy'], [hops[i] + 1.5, 'idle']]} />
          </div>
        )
      })}
      <Ripple x={BOX.x} y={BOX.y} at={c.box} size={700} rings={2} strength={0.25} beats={2} />
      {app > 0 && (
        <div style={{ position: 'absolute', left: BOX.x - 300, top: BOX.y + 150, width: 600, opacity: app, translate: `0 ${(1 - app) * 30}px` }}>
          <Panel style={{ padding: '20px 26px', borderRadius: 18, display: 'flex', alignItems: 'center', gap: 16 }}>
            <Icon name="server" size={28} />
            <div style={{ flex: 1 }}><div style={{ fontSize: 23, fontWeight: 600 }}>The app, on your server</div><div style={{ fontFamily: F.mono, fontSize: 18, color: C.text3, marginTop: 3 }}>mimir.example.com</div></div>
            <Badge live k={p(t, c.app + 0.4, 0.5, ease.back)}>Connected</Badge>
          </Panel>
        </div>
      )}
      <Gloss at={c.gloss}>Same bots, same memory, online all night. Your app connects to it.</Gloss>
    </AbsoluteFill>
  )
}

// ---------------- Recap: the six steps, then just talk ----------------

const SIX = ['Download it.', 'Pick the AI you pay for.', 'Name your first bot.', 'Put it on your phone.', 'Connect email and calendar.', 'Add a few skills.']

export function Recap() {
  const t = useBeat()
  const c = cues('recap')
  const ticks = [c.s1, c.s2, c.s3, c.s4, c.s5, c.s6]
  const out = p(t, c.talk - 0.6, 0.8, ease.in)
  return (
    <AbsoluteFill>
      {out < 1 && (
        <div style={{ position: 'absolute', inset: 0, opacity: 1 - out, filter: out ? `blur(${out * 12}px)` : undefined, translate: `0 ${-out * 50}px` }}>
          <div style={{ position: 'absolute', top: 120, left: 0, right: 0, textAlign: 'center' }}>
            <Words text="Six steps." at={c.head} step={0.25} style={{ ...headline, fontSize: 110 }} />
          </div>
          <div style={{ position: 'absolute', top: 330, left: 0, right: 0, display: 'grid', gridTemplateColumns: '640px 640px', justifyContent: 'center', columnGap: 60, rowGap: 46 }}>
            {SIX.map((s, i) => {
              const k = p(t, ticks[i], 0.6, ease.back)
              return (
                <div key={s} style={{ display: 'flex', alignItems: 'center', gap: 26, opacity: Math.min(1, k * 1.5), translate: `${(1 - k) * 24}px 0`, gridColumn: i < 3 ? 1 : 2, gridRow: (i % 3) + 1 }}>
                  <span style={{ width: 64, height: 64, flex: 'none', borderRadius: 32, display: 'grid', placeItems: 'center', fontFamily: F.serif, fontSize: 36, fontFeatureSettings: '"lnum"', color: C.deep, background: C.gall, scale: 0.7 + 0.3 * k }}>
                    {t >= ticks[i] + 0.3 ? <Icon name="check" size={34} /> : i + 1}
                  </span>
                  <span style={{ fontFamily: F.serif, fontSize: 48, color: C.foam }}>{s}</span>
                </div>
              )
            })}
          </div>
        </div>
      )}
      {t >= c.talk - 0.2 && (
        <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
          <div style={{ display: 'flex', gap: 14, marginBottom: 40 }}>
            {[BOT.ada, BOT.researcher, BOT.briefer].map((id, i) => {
              const k = p(t, c.faces + i * 0.25, 0.7, ease.back)
              return <Face key={id} id={id} size={120} moods={[[0, 'happy']]} style={{ opacity: Math.min(1, k * 2), scale: 0.5 + 0.5 * k }} />
            })}
          </div>
          <Words text="Then just *talk* *to* *it.*" at={c.talk} step={0.3} dur={1.4} style={{ ...headline, fontSize: 150 }} />
        </AbsoluteFill>
      )}
      <AbsoluteFill style={{ background: '#000', opacity: t >= c.gap ? 1 : 0 }} />
    </AbsoluteFill>
  )
}

// ---------------- End: the mark, where to get it ----------------

export function End() {
  const t = useBeat()
  const c = cues('end')
  const hit = p(t, c.hit, 2.2, ease.out)
  const url = p(t, c.url, 1.4)
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      <AbsoluteFill style={{ background: 'radial-gradient(circle at 50% 46%, rgba(223,193,154,0.22), transparent 55%)', opacity: 1 - p(t, c.hit, 2.5, ease.out) }} />
      <Ripple x={960} y={470} at={c.hit} size={2200} rings={5} strength={0.4} beats={8} />
      <div style={{ scale: 1.08 - 0.08 * hit, opacity: p(t, c.hit, 0.2, ease.lin), display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
        <Lockup id="ge" word={c.hit - 2} tag={c.tag} draw={1} fill={1} rune={[1, 1, 1, 1]} size={210} />
        <div style={{ marginTop: 54, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14, opacity: url, translate: `0 ${(1 - url) * 20}px` }}>
          <div style={{ fontFamily: F.sans, fontSize: 40, fontWeight: 500, letterSpacing: '0.01em', color: C.foam }}>heymimir.app</div>
          <div style={{ fontFamily: F.mono, fontSize: 21, letterSpacing: '0.18em', textTransform: 'uppercase', color: C.ink3 }}>Free · open source · macOS · Windows · Linux</div>
        </div>
      </div>
      <AbsoluteFill style={{ background: '#000', opacity: p(t, c.fade, 3, ease.io) }} />
    </AbsoluteFill>
  )
}

