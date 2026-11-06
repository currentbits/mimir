// ⌘K: one box to jump to a bot, a chat or a place in the app.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Icon } from './ui'

export type PaletteItem = { key: string; label: string; sub?: string; icon: ReactNode; hint?: string; run: () => void }

export function Palette({ items, onClose }: { items: PaletteItem[]; onClose: () => void }) {
  const [q, setQ] = useState('')
  const [i, setI] = useState(0)
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  const hits = items.filter(it => { const t = `${it.label} ${it.sub ?? ''}`.toLowerCase(); return words.every(w => t.includes(w)) })
  const at = Math.max(0, Math.min(i, hits.length - 1))
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => { list.current?.querySelector('.on')?.scrollIntoView({ block: 'nearest' }) }, [at])
  const pick = (it: PaletteItem) => { onClose(); it.run() }
  return (
    <div className="palette-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onClose() }}>
      <div className="palette" role="dialog" aria-label="Jump to">
        <label className="palette-bar"><Icon name="search" size={17} />
          <input autoFocus value={q} placeholder="Find a bot, chat or place" aria-label="Find a bot, chat or place" role="combobox"
            aria-expanded="true" aria-controls="palette-list" aria-activedescendant={hits[at] ? `pal-${hits[at].key}` : undefined}
            onChange={e => { setQ(e.target.value); setI(0) }}
            onKeyDown={e => {
              if (e.key === 'ArrowDown') setI(Math.min(at + 1, hits.length - 1))
              else if (e.key === 'ArrowUp') setI(Math.max(at - 1, 0))
              else if (e.key === 'Enter') { if (hits[at]) pick(hits[at]) }
              else if (e.key === 'Escape') onClose()
              else return
              // Not the sheet's Escape too, nor the textarea's caret.
              e.preventDefault(); e.stopPropagation()
            }} />
          <kbd>esc</kbd>
        </label>
        <div className="palette-list" id="palette-list" role="listbox" ref={list}>
          {hits.map((it, n) => (
            <button key={it.key} id={`pal-${it.key}`} type="button" tabIndex={-1} role="option" aria-selected={n === at}
              className={'palette-item' + (n === at ? ' on' : '')} onMouseMove={() => setI(n)} onClick={() => pick(it)}>
              <span className="pi-icon">{it.icon}</span>
              <span className="pi-text"><b>{it.label}</b>{it.sub && <small>{it.sub}</small>}</span>
              {it.hint && <kbd>{it.hint}</kbd>}
            </button>
          ))}
          {!hits.length && <p className="palette-none">Nothing matches “{q.trim()}”.</p>}
        </div>
      </div>
    </div>
  )
}
