// On your own server: bots that never sleep, a computer of their own, and skills taught by showing.
import type { CSSProperties, ReactNode } from 'react'
import { AbsoluteFill } from 'remotion'
import { BOT, C, Cursor, cues, ease, F, Face, gloss, glide, Grow, headline, Icon, lerp, p, Panel, Paper, pigment, press, Rise, Typed, useBeat, Words } from '../lib'

const ADA = pigment(BOT.ada)

// ---------------- Server: they never sleep ----------------

const CREW = [BOT.ada, BOT.researcher, BOT.watcher]
const LAP = { x: 560, y: 600 } // centre of the laptop's screen
const BOX = { x: 1330, y: 640 } // centre of the server

export function Server() {
  const t = useBeat()
  const c = cues('server')
  const lap = p(t, c.laptop, 1)
  const box = p(t, c.box, 1, ease.back)
  const lid = p(t, c.lid, 1, ease.io)
  const night = p(t, c.night, 1.5, ease.soft)
  const note = p(t, c.notify, 0.9)
  const hops = [c.hop1, c.hop2, c.hop3]
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{ background: 'rgba(4,4,12,0.45)', opacity: night }} />
      <div style={{ position: 'absolute', top: 110, left: 0, right: 0, textAlign: 'center' }}>
        <Words text="On your own server, they *never* *sleep.*" at={c.head} step={0.2} style={{ ...headline, fontSize: 92 }} />
      </div>
      {/* the laptop: its lid closes once the bots have moved out */}
      <div style={{ position: 'absolute', left: LAP.x - 260, top: LAP.y - 160, width: 520, opacity: lap, translate: `0 ${(1 - lap) * 40}px`, perspective: 1400 }}>
        <div style={{ height: 320, borderRadius: '18px 18px 4px 4px', background: '#2A2A2E', padding: 12, transformOrigin: '50% 100%', transform: `rotateX(${-lid * 86}deg)`, boxShadow: '0 30px 60px -20px rgba(0,0,0,0.8)' }}>
          <div style={{ height: '100%', borderRadius: 8, background: `linear-gradient(${C.app}, #121214)`, opacity: 1 - lid * 0.6, display: 'flex', alignItems: 'flex-end', justifyContent: 'center', padding: 18, color: C.text3, fontSize: 18, fontFamily: F.sans }}>
            {t < c.hop1 ? 'Mimir' : ''}
          </div>
        </div>
        <div style={{ height: 18, margin: '0 -30px', borderRadius: '0 0 14px 14px', background: 'linear-gradient(#3A3A3F, #232326)' }} />
        <div style={{ textAlign: 'center', marginTop: 26, opacity: p(t, c.lid + 0.6, 0.8), display: 'flex', justifyContent: 'center', alignItems: 'center', gap: 12, ...gloss, fontSize: 32 }}>
          <Icon name="moon" size={28} />your laptop sleeps
        </div>
      </div>
      {/* the server */}
      <div style={{ position: 'absolute', left: BOX.x - 190, top: BOX.y - 100, width: 380, opacity: Math.min(1, box * 1.5), scale: 0.85 + 0.15 * box }}>
        <div style={{ height: 200, borderRadius: 22, background: 'linear-gradient(#3A3229, #221D17)', boxShadow: '0 30px 60px -20px rgba(0,0,0,0.85), inset 0 1.5px 0 rgba(255,240,220,0.12)', padding: '26px 30px', display: 'flex', flexDirection: 'column', justifyContent: 'space-between' }}>
          {[0, 1, 2].map((i) => <div key={i} style={{ height: 6, borderRadius: 3, background: 'rgba(0,0,0,0.35)', boxShadow: 'inset 0 1px 0 rgba(0,0,0,0.3)' }} />)}
          <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
            {[0, 1, 2].map((i) => <span key={i} style={{ width: 11, height: 11, borderRadius: 6, background: C.live, opacity: 0.45 + 0.55 * Math.abs(Math.sin(t * 2.1 + i * 1.7)) }} />)}
            <span style={{ marginLeft: 'auto', fontFamily: F.serif, fontSize: 30, color: C.foam, opacity: 0.85 }}>ᛗ</span>
          </div>
        </div>
        <div style={{ textAlign: 'center', marginTop: 26, marginLeft: -110, marginRight: -110, whiteSpace: 'nowrap' }}>
          <div style={{ fontFamily: F.serif, fontSize: 42, color: C.foam }}>your server</div>
          <div style={{ fontFamily: F.mono, fontSize: 19, color: C.ink3, marginTop: 8, letterSpacing: '0.04em' }}>a VPS · a home server · a Raspberry Pi</div>
          <div style={{ display: 'flex', justifyContent: 'center', alignItems: 'center', gap: 10, marginTop: 14, fontFamily: F.sans, fontSize: 22, color: C.live, opacity: p(t, hops[2] + 0.8, 0.6) }}>
            <span style={{ width: 10, height: 10, borderRadius: 5, background: C.live }} />online 24/7
          </div>
        </div>
      </div>
      {/* the bots move house: from the laptop's screen to the top of the server */}
      {CREW.map((id, i) => {
        const k = p(t, hops[i], 0.9, ease.io)
        const x0 = LAP.x + (i - 1) * 96, y0 = LAP.y - 30, x1 = BOX.x + (i - 1) * 96, y1 = BOX.y - 137
        const shown = p(t, c.laptop + 0.4 + i * 0.15, 0.6, ease.back)
        return (
          <div key={id} style={{ position: 'absolute', left: lerp(x0, x1, k) - 41, top: lerp(y0, y1, k) - Math.sin(k * Math.PI) * 220 - 41, scale: shown * (k > 0 && k < 1 ? 1.06 : 1), opacity: lid > 0.5 && k === 0 ? 0 : 1, zIndex: 3 }}>
            <Face id={id} size={82} moods={[[0, 'idle'], [hops[i] - 0.2, 'happy'], [hops[i] + 1.4, 'idle'], [c.notify - 0.6, id === BOT.watcher ? 'attention' : 'idle']]} />
          </div>
        )
      })}
      {/* 3:12 at night, and a bot still has news */}
      {note > 0 && (
        <div style={{ position: 'absolute', left: LAP.x - 330, top: 560, opacity: note, translate: `0 ${(1 - note) * 40}px` }}>
          <Panel style={{ width: 660, borderRadius: 26, padding: '20px 24px', display: 'flex', gap: 18, alignItems: 'center', background: 'rgba(44,44,49,0.97)' }}>
            <Face id={BOT.watcher} size={60} moods={[[0, 'writing']]} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 22 }}><b style={{ fontWeight: 600 }}>Page watcher</b><span style={{ color: C.text3, fontFamily: F.mono }}>03:12</span></div>
              <div style={{ fontSize: 22, color: C.text2, marginTop: 3 }}>The flat on Kloof Street is listed. Ask for a viewing?</div>
            </div>
          </Panel>
        </div>
      )}
    </AbsoluteFill>
  )
}

// ---------------- the Computer window, shared by the two scenes after it ----------------

type Step = { icon: string; text: ReactNode; at: number; quiet?: boolean }

function Steps({ title, face, steps }: { title: string; face: ReactNode; steps: Step[] }) {
  const t = useBeat()
  return (
    <div style={{ width: 430, flex: 'none', background: '#1D1D20', borderLeft: `1px solid ${C.line}`, padding: '22px 22px', display: 'flex', flexDirection: 'column' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontSize: 21, color: C.text2, paddingBottom: 14, borderBottom: `1px solid ${C.line}` }}>{face}{title}</div>
      {steps.map((s, i) => {
        const k = p(t, s.at, 0.6)
        return (
          <Grow key={i} k={k}>
            <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start', padding: '14px 2px 0', opacity: k, color: s.quiet ? C.text3 : C.text }}>
              <span style={{ color: s.quiet ? C.pending : C.text3, display: 'flex', paddingTop: 2 }}><Icon name={s.icon} size={22} /></span>
              <span style={{ fontSize: 22, lineHeight: 1.35 }}>{s.text}</span>
            </div>
          </Grow>
        )
      })}
    </div>
  )
}

/** The Computer page: the bots' Linux desktop with Chromium, its status, and the steps beside it. */
function ComputerWindow({ status, action, url, children, side, scan = -1 }: { status: ReactNode; action: ReactNode; url: string; children: ReactNode; side: ReactNode; scan?: number }) {
  return (
    <Panel style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <div style={{ height: 64, flex: 'none', display: 'flex', alignItems: 'center', gap: 16, padding: '0 20px 0 24px', background: '#1E1E21', borderBottom: `1px solid ${C.line}` }}>
        <span style={{ color: C.text2, display: 'flex' }}><Icon name="laptop" size={26} /></span>
        <span style={{ fontFamily: F.ui, fontSize: 26 }}>Computer</span>
        <span style={{ fontFamily: F.mono, fontSize: 17, color: C.text3 }}>on your server</span>
        <span style={{ marginLeft: 26, display: 'flex', alignItems: 'center', gap: 10, fontSize: 21, color: C.text2 }}>{status}</span>
        <span style={{ marginLeft: 'auto' }}>{action}</span>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <div style={{ position: 'relative', flex: 1, background: 'linear-gradient(160deg, #3B4048, #23262B)', padding: 22 }}>
          <div style={{ height: '100%', borderRadius: 10, overflow: 'hidden', background: '#FBFBFA', display: 'flex', flexDirection: 'column', boxShadow: '0 12px 30px rgba(0,0,0,0.4)' }}>
            <div style={{ height: 42, flex: 'none', background: '#DEE1E6', display: 'flex', alignItems: 'flex-end', padding: '0 10px' }}>
              <div style={{ height: 34, width: 250, borderRadius: '10px 10px 0 0', background: '#fff', fontSize: 16, color: '#3C4043', display: 'flex', alignItems: 'center', padding: '0 14px', fontFamily: F.sans }}>{url.split('/')[0]}</div>
            </div>
            <div style={{ height: 46, flex: 'none', background: '#fff', borderBottom: '1px solid #E3E3E6', display: 'flex', alignItems: 'center', padding: '0 14px' }}>
              <div style={{ flex: 1, height: 32, borderRadius: 16, background: '#F1F3F4', fontSize: 17, color: '#3C4043', display: 'flex', alignItems: 'center', padding: '0 16px', fontFamily: F.sans }}>{url}</div>
            </div>
            <div style={{ position: 'relative', flex: 1, color: '#1C1C1E', fontFamily: F.sans }}>{children}</div>
          </div>
          {scan >= 0 && scan < 1 && (
            <div style={{ position: 'absolute', left: 22, right: 22, top: `calc(22px + ${scan * 90}%)`, height: 140, background: 'linear-gradient(transparent, rgba(223,193,154,0.28), transparent)', opacity: Math.sin(scan * Math.PI) }} />
          )}
        </div>
        {side}
      </div>
    </Panel>
  )
}

function Frame({ title, gl, gl2, c, children }: { title: string; gl: string; gl2?: [string, number]; c: Record<string, number>; children: ReactNode }) {
  const t = useBeat()
  const win = p(t, c.win ?? 0, 1.2)
  return (
    <AbsoluteFill style={{ padding: '84px 150px 80px' }}>
      <div style={{ display: 'flex', alignItems: 'flex-end', justifyContent: 'space-between', gap: 60 }}>
        <Words text={title} at={c.head} step={0.2} style={{ ...headline, fontSize: 88, whiteSpace: 'nowrap' }} />
        <div style={{ position: 'relative', width: 600, height: 96 }}>
          <div style={{ position: 'absolute', inset: 0, opacity: gl2 ? 1 - p(t, gl2[1] - 0.4, 0.4, ease.in) : 1 }}>
            <Rise at={c.gloss}><p style={{ ...gloss, fontSize: 32, textAlign: 'right' }}>{gl}</p></Rise>
          </div>
          {gl2 && <div style={{ position: 'absolute', inset: 0 }}><Rise at={gl2[1]}><p style={{ ...gloss, fontSize: 32, textAlign: 'right' }}>{gl2[0]}</p></Rise></div>}
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0, marginTop: 36, opacity: win, translate: `0 ${(1 - win) * 60}px` }}>{children}</div>
    </AbsoluteFill>
  )
}
const pill = (bg: string, fg: string): CSSProperties => ({ fontSize: 20, padding: '9px 18px', borderRadius: 10, background: bg, color: fg, fontFamily: F.sans, fontWeight: 500 })

// ---------------- Computer: it uses any website like a person; you can take over ----------------

function Field({ label, children, focus, color = ADA }: { label: string; children: ReactNode; focus?: boolean; color?: string }) {
  return (
    <div style={{ marginTop: 18 }}>
      <div style={{ fontSize: 19, color: '#5D5D64', marginBottom: 8, height: 24 }}>{label}</div>
      <div style={{ height: 58, borderRadius: 12, border: `2px solid ${focus ? color : '#DDDDE0'}`, padding: '0 18px', display: 'flex', alignItems: 'center', fontSize: 24, background: '#fff' }}>{children}</div>
    </div>
  )
}

const SLOTS = ['17:00', '18:00', '18:30', '19:30']

export function Computer() {
  const t = useBeat()
  const c = cues('computer')
  const yours = t >= c.take && t < c.give
  const page2 = t >= c.page
  const pg = p(t, c.page, 0.6)
  const booked = t >= c.slot
  // Ada's pointer (page coordinates): to the email field, Sign in, then the 18:30 slot on court 2.
  const A: [number, number] = [860, 520], E: [number, number] = [420, 205], S: [number, number] = [578, 397], G: [number, number] = [773, 281]
  const ada = t < c.signin - 1 ? glide(t, c.open, 0.8, A, E) : t < c.page ? glide(t, c.pass + 0.2, c.signin - c.pass - 0.4, E, S) : glide(t, c.page + 0.4, c.slot - c.page - 0.7, S, G)
  // Your pointer (window coordinates): in to Take over, about the page, then Give back.
  const me = t < c.take ? glide(t, c.take - 1.4, 1.1, [1500, 700], [1508, 32]) : t < c.give - 1.2 ? glide(t, c.take + 0.4, 1.4, [1508, 32], [900, 420]) : glide(t, c.give - 1.1, 0.9, [900, 420], [1508, 32])
  const steps: Step[] = [
    { icon: 'search', text: 'Looked at the screen', at: c.look },
    { icon: 'globe', text: 'Opened playtomic.io (you allowed this site)', at: c.open },
    { icon: 'key', text: 'Signed in with your saved login. The password never reaches the AI.', at: c.pass },
    { icon: 'check', text: 'Booked Court 2, Thursday 18:30', at: c.booked },
    { icon: 'user', text: 'You took over. Bots wait.', at: c.take + 0.2, quiet: true },
    { icon: 'retry', text: 'You gave it back.', at: c.give + 0.2, quiet: true },
  ]
  return (
    <Frame title="And a computer of their own." gl="It sees the screen and uses the mouse and keyboard, like a person. Any website works." gl2={['Watch it live. Take over any time.', c.gloss2]} c={c}>
      <div style={{ position: 'relative', height: '100%' }}>
        <ComputerWindow url={page2 ? 'playtomic.io/clubs/sea-point-padel' : 'playtomic.io/login'} scan={p(t, c.look, 1.2, ease.io)}
          status={yours
            ? <><span style={{ width: 11, height: 11, borderRadius: 6, background: C.pending }} />You have control · bots wait</>
            : <><span style={{ width: 11, height: 11, borderRadius: 6, background: C.live }} /><Face id={BOT.ada} size={30} moods={[[0, 'reading'], [c.booked, 'happy'], [c.booked + 1.5, 'idle']]} />Ada is using it</>}
          action={<span style={{ ...pill(yours ? C.text : 'rgba(255,255,255,0.1)', yours ? C.app : C.text), display: 'inline-block', scale: 1 - (press(t, c.take) + press(t, c.give)) * 0.06 }}>{yours ? 'Give back' : 'Take over'}</span>}
          side={<Steps title="What Ada did" face={<Face id={BOT.ada} size={34} />} steps={steps} />}>
          {(!page2 || pg < 1) && (
            <div style={{ position: 'absolute', inset: 0, opacity: (1 - pg) * p(t, c.open - 0.2, 0.5) }}>
              <div style={{ width: 480, margin: '34px auto 0' }}>
                <div style={{ fontSize: 26, fontWeight: 800, letterSpacing: '-0.02em', height: 32 }}>playtomic</div>
                <div style={{ fontSize: 38, fontWeight: 600, marginTop: 14, height: 46 }}>Sign in</div>
                <Field label="Email" focus={t >= c.open + 0.8 && t < c.pass}><Typed text="you@example.com" at={c.open + 0.8} dur={0.7} caret={false} /></Field>
                <Field label="Password" focus={t >= c.pass && t < c.signin}>
                  <span style={{ letterSpacing: 4, opacity: p(t, c.pass, 0.3) }}>••••••••••••</span>
                  {t >= c.pass && <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8, fontSize: 16, color: '#2B8E59' }}><Icon name="key" size={17} />saved login</span>}
                </Field>
                <div style={{ marginTop: 26, height: 58, borderRadius: 12, background: '#1C1C1E', color: '#fff', display: 'grid', placeItems: 'center', fontSize: 24, fontWeight: 600, scale: 1 - press(t, c.signin) * 0.04 }}>Sign in</div>
              </div>
            </div>
          )}
          {page2 && (
            <div style={{ position: 'absolute', inset: 0, padding: '34px 50px', opacity: pg }}>
              <div style={{ fontSize: 34, fontWeight: 600, height: 40 }}>Sea Point Padel</div>
              <div style={{ fontSize: 21, color: '#5D5D64', height: 30 }}>Thursday · 90 minutes</div>
              <div style={{ display: 'grid', gridTemplateColumns: '150px repeat(4, 1fr)', gridAutoRows: 'auto', gap: 14, marginTop: 28, fontSize: 21 }}>
                <span style={{ height: 28 }} />{SLOTS.map((s) => <span key={s} style={{ textAlign: 'center', color: '#5D5D64', height: 28 }}>{s}</span>)}
                {[1, 2, 3].flatMap((court) => [
                  <span key={`c${court}`} style={{ alignSelf: 'center', color: '#5D5D64' }}>Court {court}</span>,
                  ...SLOTS.map((s, i) => {
                    const taken = (court + i) % 3 === 0
                    const mine = court === 2 && s === '18:30'
                    return (
                      <span key={`${court}${s}`} style={{ height: 62, borderRadius: 12, display: 'grid', placeItems: 'center', fontSize: 20,
                        background: mine && booked ? ADA : taken ? '#ECECEE' : '#fff', color: mine && booked ? '#fff' : taken ? '#A0A0A6' : '#1C1C1E',
                        border: `2px solid ${mine && booked ? ADA : '#E2E2E5'}`, scale: mine && booked ? 0.92 + 0.08 * p(t, c.slot, 0.5, ease.back) : 1 }}>
                        {mine && booked ? <Icon name="check" size={26} /> : taken ? 'Taken' : 'Free'}
                      </span>
                    )
                  }),
                ])}
              </div>
            </div>
          )}
          {!yours && t < c.take && <Cursor x={ada.x} y={ada.y} press={press(t, t < c.page ? c.signin : c.slot)} color={ADA}
            tag={<span style={{ fontSize: 16, fontWeight: 600, color: '#fff', background: ADA, padding: '3px 9px', borderRadius: 7 }}>Ada</span>} />}
        </ComputerWindow>
        {t >= c.take - 1.4 && t < c.give + 1 && <Cursor x={me.x} y={me.y} press={press(t, c.take) + press(t, c.give)}
          tag={<span style={{ fontSize: 16, fontWeight: 600, color: C.app, background: C.text, padding: '3px 9px', borderRadius: 7 }}>You</span>} />}
      </div>
    </Frame>
  )
}

// ---------------- Teach: do it once, and it drafts the skill ----------------

const USUALS = ['Oat milk, 2 litres', 'Sourdough loaf', 'Free-range eggs, 12', 'Coffee beans, 500 g']

export function Teach() {
  const t = useBeat()
  const c = cues('teach')
  const showing = t >= c.start
  const added = t >= c.c1
  const secs = Math.max(0, Math.floor(((t - c.start) * 60) / 100))
  const skill = p(t, c.skill, 1.1, ease.out)
  // Your pointer (page coordinates, then the window's top bar for Done).
  const page = t < c.c1 ? glide(t, c.start + 0.1, 0.9, [600, 520], [140, 372]) : t < c.c2 ? glide(t, c.c1 + 0.3, c.c2 - c.c1 - 0.6, [140, 372], [800, 109]) : t < c.card ? { x: 800, y: 109 } : glide(t, c.card - 0.5, 0.4, [800, 109], [800, 217])
  const bar = glide(t, c.card + 0.6, c.done - c.card - 0.9, [1060, 520], [1468, 32])
  const steps: Step[] = [
    { icon: 'check', text: 'Clicked “Add all to cart”', at: c.c1 },
    { icon: 'pencil', text: 'Clicked “Delivery note”', at: c.c2 },
    { icon: 'pencil', text: 'Typed “Leave at the door”', at: c.type + 0.9 },
    { icon: 'key', text: 'Card details: not kept', at: c.card + 0.3, quiet: true },
  ]
  return (
    <Frame title="Teach it by showing." gl="Do it once. Mimir writes the steps down and drafts a skill for you to accept." c={{ ...c, win: -2 }}>
      <div style={{ position: 'relative', height: '100%' }}>
        <ComputerWindow url="greenleaf.example/account/usuals"
          status={showing
            ? <><span style={{ width: 12, height: 12, borderRadius: 6, background: C.danger, opacity: 0.5 + 0.5 * Math.cos(t * Math.PI) ** 2 }} />Showing · 0:{String(secs).padStart(2, '0')}</>
            : <><span style={{ width: 11, height: 11, borderRadius: 6, background: C.text3 }} />Nobody is using it</>}
          action={<span style={{ ...pill(showing ? C.text : 'rgba(255,255,255,0.1)', showing ? C.app : C.text), display: 'inline-block', scale: 1 - (press(t, c.start) + press(t, c.done)) * 0.06 }}>{showing ? 'Done, draft a skill' : 'Start showing'}</span>}
          side={<Steps title="Ada is watching" face={<Face id={BOT.ada} size={34} moods={[[0, 'reading'], [c.done, 'writing']]} />} steps={steps} />}>
          <div style={{ position: 'absolute', inset: 0, padding: '30px 46px', display: 'flex', gap: 46 }}>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 24, fontWeight: 800, color: '#2F6B3A', letterSpacing: '-0.01em' }}>greenleaf</div>
              <div style={{ fontSize: 32, fontWeight: 600, marginTop: 14 }}>Your usuals</div>
              <div style={{ marginTop: 14 }}>
                {USUALS.map((u) => (
                  <div key={u} style={{ display: 'flex', alignItems: 'center', gap: 14, height: 50, borderBottom: '1px solid #ECECEE', fontSize: 21 }}>
                    <span style={{ width: 24, height: 24, borderRadius: 6, border: `2px solid ${added ? '#2F6B3A' : '#CFCFD4'}`, background: added ? '#2F6B3A' : '#fff', color: '#fff', display: 'grid', placeItems: 'center' }}>{added && <Icon name="check" size={16} />}</span>{u}
                  </div>
                ))}
              </div>
              <div style={{ marginTop: 20, display: 'inline-block', padding: '13px 22px', borderRadius: 12, background: '#2F6B3A', color: '#fff', fontSize: 21, fontWeight: 600, scale: 1 - press(t, c.c1) * 0.05 }}>{added ? 'Added to cart' : 'Add all to cart'}</div>
            </div>
            <div style={{ width: 340 }}>
              <Field label="Delivery note" focus={t >= c.c2 && t < c.card} color="#1C1C1E"><Typed text="Leave at the door" at={c.type} dur={0.9} caret={false} /></Field>
              <Field label="Card number" focus={t >= c.card} color="#1C1C1E"><span style={{ letterSpacing: 2, opacity: p(t, c.card, 0.3) }}>•••• •••• •••• 4242</span></Field>
            </div>
          </div>
          {showing && t < c.card + 0.6 && <Cursor x={page.x} y={page.y} press={press(t, c.c1) + press(t, c.c2)} tag={<span style={{ fontSize: 16, fontWeight: 600, color: C.app, background: C.text, padding: '3px 9px', borderRadius: 7 }}>You</span>} />}
        </ComputerWindow>
        {t >= c.card + 0.6 && t < c.done + 0.6 && <Cursor x={bar.x} y={bar.y} press={press(t, c.done)} tag={<span style={{ fontSize: 16, fontWeight: 600, color: C.app, background: C.text, padding: '3px 9px', borderRadius: 7 }}>You</span>} />}
        {skill > 0 && (
          <div style={{ position: 'absolute', left: '50%', top: 70, translate: `-50% ${(1 - skill) * 80}px`, opacity: skill, rotate: `${(1 - skill) * -3}deg` }}>
            <Paper style={{ width: 760, padding: '34px 42px 30px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontFamily: F.mono, fontSize: 18, letterSpacing: '0.18em', textTransform: 'uppercase', color: C.pInk3 }}><Icon name="wand" size={22} />Skill · proposed</div>
              <div style={{ fontFamily: F.serif, fontSize: 52, lineHeight: 1.05, marginTop: 10 }}>Order the usual groceries</div>
              <div style={{ fontFamily: F.mono, fontSize: 18, color: C.pInk3, marginTop: 8 }}>drafted by Ada from what you showed</div>
              {['Open greenleaf.example, your usuals', 'Add all to cart', 'Write {delivery note} in Delivery note', 'Ask before paying'].map((s, i) => (
                <div key={s} style={{ display: 'flex', gap: 18, alignItems: 'baseline', paddingTop: 14, opacity: p(t, c.skill + 0.4 + i * 0.25, 0.5) }}>
                  <span style={{ fontFamily: F.serif, fontSize: 32, color: '#6E5135', width: 24 }}>{i + 1}</span>
                  <span style={{ fontSize: 27 }}>{s}</span>
                </div>
              ))}
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 22 }}>
                <span style={{ fontSize: 24, padding: '12px 18px', color: C.pInk2 }}>Dismiss</span>
                <span style={{ fontSize: 24, padding: '12px 26px', borderRadius: 12, background: C.pInk, color: C.paper }}>Accept</span>
              </div>
            </Paper>
          </div>
        )}
      </div>
    </Frame>
  )
}
