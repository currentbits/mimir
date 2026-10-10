// The build: on your AI, yours alone, everywhere and free. Quicker cuts, on the beat.
import { AbsoluteFill } from 'remotion'
import { C, cues, ease, F, gloss, headline, Icon, p, Rise, Typed, useBeat, Words } from '../lib'

const AIS = ['Claude', 'ChatGPT', 'Codex', 'Grok', 'OpenRouter', 'Ollama', 'any API key']

export function Ai() {
  const t = useBeat()
  const c = cues('ai')
  // The slot rolls one name per half beat, easing into each.
  const step = Math.max(0, Math.min(AIS.length - 1, (t - c.cycle) / 0.5))
  const pos = Math.floor(step) + ease.out(Math.min(1, (step % 1) * 1.8))
  const on = p(t, c.cycle - 0.4, 0.5)
  return (
    <AbsoluteFill style={{ alignItems: 'center' }}>
      <div style={{ marginTop: 210, textAlign: 'center' }}>
        <Words text={'Runs on the AI\nyou already pay for.'} at={c.head} step={0.2} style={{ ...headline, fontSize: 112 }} />
      </div>
      <div style={{ position: 'relative', width: 1200, height: 260, marginTop: 40, opacity: on, maskImage: 'linear-gradient(transparent, #000 30%, #000 70%, transparent)' }}>
        {AIS.map((name, i) => {
          const d = i - pos
          if (Math.abs(d) > 1.6) return null
          return (
            <div key={name} style={{ position: 'absolute', left: 0, right: 0, top: 130 - 75 + d * 130, textAlign: 'center', fontFamily: F.serif, fontStyle: 'italic', fontSize: 120, lineHeight: '150px', color: C.gall, opacity: 1 - Math.min(1, Math.abs(d)) * 0.75, filter: Math.abs(d) > 0.05 ? `blur(${Math.abs(d) * 6}px)` : undefined }}>{name}</div>
          )
        })}
      </div>
      <div style={{ position: 'absolute', bottom: 100 }}>
        <Rise at={c.gloss}><p style={gloss}>Mimir has no AI of its own. It uses yours.</p></Rise>
      </div>
    </AbsoluteFill>
  )
}

const cut = (t: number, at: number) => ({ opacity: p(t, at, 0.15, ease.lin), scale: 1.04 - 0.04 * p(t, at, 0.8) })

export function Yours() {
  const t = useBeat()
  const c = cues('yours')
  const gone = p(t, c.file - 0.4, 0.6, ease.in)
  const file = p(t, c.file, 0.9, ease.back)
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      {gone < 1 && (
        <div style={{ position: 'absolute', textAlign: 'center', opacity: 1 - gone, filter: gone ? `blur(${gone * 14}px)` : undefined, scale: 1 - gone * 0.1 }}>
          <div style={{ ...headline, fontSize: 132, ...cut(t, c.l1) }}>No Mimir account.</div>
          <div style={{ ...headline, fontSize: 132, marginTop: 10, ...cut(t, c.l2) }}>No Mimir server.</div>
        </div>
      )}
      {file > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 28, opacity: Math.min(1, file * 1.5), scale: 0.8 + 0.2 * file, color: C.foam }}>
            <Icon name="file" size={120} />
            <div>
              <div style={{ fontFamily: F.mono, fontSize: 44 }}>mimir.db</div>
              <div style={{ fontFamily: F.sans, fontSize: 26, color: C.ink3, marginTop: 8, display: 'flex', alignItems: 'center', gap: 10 }}><Icon name="key" size={24} />secrets in your keychain</div>
            </div>
          </div>
          <Words text={'What it knows about you\n*stays* *with* *you.*'} at={c.stays} step={0.18} style={{ ...headline, fontSize: 104, textAlign: 'center', marginTop: 70 }} />
        </div>
      )}
    </AbsoluteFill>
  )
}

const OS = ['macOS', 'Windows', 'Linux', 'your own server']

export function Everywhere() {
  const t = useBeat()
  const c = cues('everywhere')
  const at = [c.os1, c.os2, c.os3, c.os4]
  const up = p(t, c.free - 0.3, 0.8, ease.in)
  const free = p(t, c.free, 1.2)
  return (
    <AbsoluteFill style={{ justifyContent: 'center', alignItems: 'center' }}>
      {up < 1 && (
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 60, opacity: 1 - up, translate: `0 ${-up * 80}px`, filter: up ? `blur(${up * 12}px)` : undefined }}>
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 40, ...headline, fontSize: 92 }}>
            {OS.map((o, i) => (
              <span key={o} style={{ display: 'flex', alignItems: 'baseline', gap: 40, ...cut(t, at[i]), fontStyle: i === 3 ? 'italic' : undefined, color: i === 3 ? C.gall : undefined }}>
                {i > 0 && <span style={{ color: C.ink3, fontSize: 50 }}>·</span>}{o}
              </span>
            ))}
          </div>
          <div style={{ fontFamily: F.mono, fontSize: 38, color: C.foam, padding: '20px 34px', borderRadius: 16, background: 'rgba(0,0,0,0.45)', boxShadow: 'inset 0 0 0 1px rgba(236,233,225,0.12)', opacity: p(t, c.term - 0.3, 0.4), minWidth: 560 }}>
            <span style={{ color: C.ink3 }}>$ </span><Typed text="docker compose up -d" at={c.term} dur={1} />
          </div>
        </div>
      )}
      {free > 0 && (
        <div style={{ position: 'absolute', textAlign: 'center', opacity: free, scale: 1.06 - 0.06 * free }}>
          <div style={{ ...headline, fontSize: 150 }}>Free and <i style={{ color: C.gall }}>open source.</i></div>
          <Rise at={c.free + 1}><div style={{ fontFamily: F.mono, fontSize: 24, letterSpacing: '0.16em', textTransform: 'uppercase', color: C.ink3, marginTop: 34 }}>AGPL-3.0 · github.com/currentbits/mimir</div></Rise>
        </div>
      )}
      <AbsoluteFill style={{ background: '#000', opacity: t >= c.gap ? 1 : 0 }} />
    </AbsoluteFill>
  )
}
