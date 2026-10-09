// Every bot face and mood on one page, for working on them: npm run dev, then open /faces.html. Not in the app build.
import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import './index.css'
import { Avatar } from './ui'
import type { Mood } from './face'

const BOTS = [[1, 'Ada'], [2, 'Researcher'], [3, 'Morning briefer'], [4, 'Page watcher'], [5, 'Weekly reviewer'], [6, 'Follow-up keeper']] as const
const MOODS: [Mood, string][] = [['idle', 'idle: breathes, blinks, glances'], ['listening', 'listening: you are typing'], ['thinking', 'thinking'], ['searching', 'searching memory or the web'],
  ['reading', 'reading a page'], ['writing', 'writing the reply'], ['asking', 'waiting for your OK'], ['remembering', 'saving to memory'],
  ['happy', 'done'], ['surprised', 'stopped'], ['sad', 'something failed'], ['asleep', 'asleep'], ['attention', 'needs you']]

function Gallery() {
  const [text, setText] = useState('')
  const [bump, setBump] = useState(0)
  return (
    <div style={{ padding: 40, display: 'grid', gap: 36, font: '14px system-ui', color: 'var(--text)', background: 'var(--bg)', minHeight: '100vh' }}>
      <section>
        <h2 style={{ font: '28px var(--serif)', margin: '0 0 18px' }}>Every bot looks like itself</h2>
        <div style={{ display: 'flex', gap: 34 }}>
          {BOTS.map(([id, name]) => <figure key={id} style={{ margin: 0, display: 'grid', justifyItems: 'center', gap: 10 }}><Avatar id={id} name={name} size={64} alive /><figcaption>{name}</figcaption></figure>)}
        </div>
      </section>
      <section>
        <h2 style={{ font: '28px var(--serif)', margin: '0 0 18px' }}>What they do</h2>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(7, 130px)', gap: '26px 10px' }} key={bump}>
          {MOODS.map(([m, label]) => (
            <figure key={m} style={{ margin: 0, display: 'grid', justifyItems: 'center', gap: 10, textAlign: 'center' }}>
              <div style={{ display: 'flex', gap: 12 }}><Avatar id={1} name="Ada" size={52} mood={m} /><Avatar id={2} name="Researcher" size={52} mood={m} /></div>
              <figcaption style={{ fontSize: 12, color: 'var(--text-2)' }}>{label}</figcaption>
            </figure>
          ))}
        </div>
        <button className="btn btn-sm" style={{ marginTop: 18 }} onClick={() => setBump(b => b + 1)}>Replay one-off reactions</button>
      </section>
      <section>
        <h2 style={{ font: '28px var(--serif)', margin: '0 0 18px' }}>Type to it</h2>
        <div className="composer" style={{ maxWidth: 520, display: 'flex', alignItems: 'center', gap: 10, padding: 8 }}>
          <Avatar id={3} name="Morning briefer" size={28} alive beat={text.length} mood={text.trim() ? 'listening' : 'idle'} />
          <input className="input" style={{ flex: 1 }} placeholder="Message Morning briefer" value={text} onChange={e => setText(e.target.value)} />
        </div>
      </section>
      <section>
        <h2 style={{ font: '28px var(--serif)', margin: '0 0 18px' }}>At the sizes the app uses</h2>
        <div style={{ display: 'flex', gap: 22, alignItems: 'end' }}>
          {[16, 18, 26, 28, 32, 56].map(s => <figure key={s} style={{ margin: 0, display: 'grid', justifyItems: 'center', gap: 8 }}><Avatar id={1} name="Ada" size={s} /><figcaption style={{ fontSize: 11, color: 'var(--text-3)' }}>{s}</figcaption></figure>)}
          {[16, 18, 26, 28, 32, 56].map(s => <figure key={'b' + s} style={{ margin: 0, display: 'grid', justifyItems: 'center', gap: 8 }}><Avatar id={2} name="Researcher" size={s} presence="live" /><figcaption style={{ fontSize: 11, color: 'var(--text-3)' }}>{s}</figcaption></figure>)}
        </div>
      </section>
    </div>
  )
}
createRoot(document.getElementById('root')!).render(<Gallery />)
