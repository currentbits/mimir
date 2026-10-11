// Cards a bot keeps up to date, pinned at the top of a chat: a shopping list, bills due, a project's state.
// Text, checklists, tables and links only (the Markdown renderer never builds HTML). A tick writes back as your edit.
import { useCallback, useEffect, useState } from 'react'
import { invoke } from './api'
import { ago, fullTime, pigment } from './format'
import { ask, Icon, tell } from './ui'
import { Markdown } from './markdown'

type Card = { id: number; bot_id: number; chat_id: number; routine_id: number | null; title: string; body: string; by_you: boolean; updated_at: number }

export function PinnedCards({ chatId, botId, botName }: { chatId: number; botId?: number; botName: (id: number) => string }) {
  const [cards, setCards] = useState<Card[]>([])
  const load = useCallback(() => invoke<Card[]>('cards', { chatId }).then(setCards, () => {}), [chatId])
  useEffect(() => { void load(); const t = setInterval(load, 10000); return () => clearInterval(t) }, [load])
  const shown = cards.filter(k => botId === undefined || k.bot_id === botId)
  if (!shown.length) return null
  const tick = (k: Card) => (line: number, text: string) =>
    invoke<Card>('card_tick', { chatId, id: k.id, line, text }).then(n => setCards(cs => cs.map(c => c.id === n.id ? n : c)), e => { tell(String(e)); void load() })
  const remove = (k: Card) => ask(`Remove the card “${k.title}”?${k.routine_id ? ' Its routine makes a new one on its next run.' : ''}`, 'Remove', true)
    .then(ok => { if (ok) void invoke('card_delete', { chatId, id: k.id }).then(load, e => tell(`Couldn't remove it: ${e}`)) })
  return (
    <div className="pinned-cards">
      {shown.map(k => (
        <details key={k.id} className="pinned-card" open style={{ '--pig': pigment(k.bot_id) } as React.CSSProperties}>
          <summary>
            <Icon name="pin" size={13} /><span className="pinned-title">{k.title}</span>
            <span className="mono hint" title={fullTime(k.updated_at)}>{k.by_you ? 'you' : botName(k.bot_id)} · {ago(k.updated_at)}</span>
            <button className="btn btn-ghost btn-danger btn-sm" title="Remove this card" aria-label="Remove this card" onClick={e => { e.preventDefault(); void remove(k) }}><Icon name="trash" size={13} /></button>
          </summary>
          <Markdown text={k.body} onTick={tick(k)} />
        </details>
      ))}
    </div>
  )
}
