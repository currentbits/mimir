// Cards a bot keeps up to date, pinned at the top of a chat: a shopping list, bills due, a project's state.
// Text, checklists, tables and links only (the Markdown renderer never builds HTML). A tick writes back as your edit.
import { useCallback, useEffect, useState } from 'react'
import { invoke } from '../api'
import { ago, fullTime, pigment } from '../format'
import { ask, Icon, tell } from '../ui'
import { Markdown } from '../markdown'
import type { Goal } from '../types'

type Card = { id: number; bot_id: number; chat_id: number; routine_id: number | null; title: string; body: string; by_you: boolean; updated_at: number }

const tokens = (n: number) => n >= 10000 ? `${Math.round(n / 1000)}k` : String(n)

/** The chat's goal, polled like its cards. */
function useGoal(chatId: number) {
  const [goal, setGoal] = useState<Goal | null>(null)
  const load = useCallback(() => invoke<Goal | null>('goal', { chatId }).then(setGoal, () => {}), [chatId])
  useEffect(() => { void load(); const t = setInterval(load, 10000); return () => clearInterval(t) }, [load])
  return { goal, setGoal, load }
}

/** One line for the chat's header: the goal and how it stands. */
export function GoalLine({ chatId }: { chatId: number }) {
  const { goal } = useGoal(chatId)
  if (!goal) return null
  return <> · <span title={`Done when ${goal.done_when}`}>Goal{goal.status === 'active' ? '' : ` (${goal.status})`}: {goal.text}</span></>
}

/** The goal pinned above the cards: yours to pause, resume, mark done or remove, whatever the bot is doing. */
function GoalCard({ chatId, botName }: { chatId: number; botName: (id: number) => string }) {
  const { goal: g, setGoal, load } = useGoal(chatId)
  if (!g) return null
  const status = (s: Goal['status']) => invoke<Goal>('goal_status', { chatId, status: s }).then(setGoal, e => tell(String(e)))
  const remove = () => ask(`Remove the goal “${g.text}”? The follow-ups and routines it booked go with it, and tasks it handed to other bots stop.`, 'Remove', true)
    .then(ok => { if (ok) void invoke('goal_delete', { chatId }).then(load, e => tell(`Couldn't remove it: ${e}`)) })
  const on = g.status === 'active' || g.status === 'blocked'
  return (
    <details className="pinned-card" open style={{ '--pig': pigment(g.bot_id) } as React.CSSProperties}>
      <summary>
        <Icon name="bolt" size={13} /><span className="pinned-title">{g.text}</span>
        <span className="mono hint" title={fullTime(g.updated_at)}>{g.status} · {botName(g.bot_id)} · {ago(g.updated_at)}</span>
        {on && <button className="btn btn-ghost btn-sm" title="Pause: its follow-ups and routines wait, and tasks it handed over stop" aria-label="Pause the goal" onClick={e => { e.preventDefault(); void status('paused') }}><Icon name="pause" size={13} /></button>}
        {g.status === 'paused' && <button className="btn btn-ghost btn-sm" title="Go on (its budget starts over)" aria-label="Resume the goal" onClick={e => { e.preventDefault(); void status('active') }}><Icon name="play" size={13} /></button>}
        {g.status !== 'done' && <button className="btn btn-ghost btn-sm" title="It's done: its follow-ups and routines are removed" aria-label="Mark the goal done" onClick={e => { e.preventDefault(); void status('done') }}><Icon name="check" size={13} /></button>}
        <button className="btn btn-ghost btn-danger btn-sm" title="Remove this goal" aria-label="Remove this goal" onClick={e => { e.preventDefault(); void remove() }}><Icon name="trash" size={13} /></button>
      </summary>
      <p className="hint">Done when {g.done_when}.{g.budget > 0 && ` ${tokens(g.spent)} of ${tokens(g.budget)} tokens used.`}{g.why && ` ${g.why[0].toUpperCase()}${g.why.slice(1)}.`}</p>
      {g.progress && <Markdown text={g.progress} />}
    </details>
  )
}

export function PinnedCards({ chatId, botId, botName }: { chatId: number; botId?: number; botName: (id: number) => string }) {
  const [cards, setCards] = useState<Card[]>([])
  const load = useCallback(() => invoke<Card[]>('cards', { chatId }).then(setCards, () => {}), [chatId])
  useEffect(() => { void load(); const t = setInterval(load, 10000); return () => clearInterval(t) }, [load])
  const shown = cards.filter(k => botId === undefined || k.bot_id === botId)
  const tick = (k: Card) => (line: number, text: string) =>
    invoke<Card>('card_tick', { chatId, id: k.id, line, text }).then(n => setCards(cs => cs.map(c => c.id === n.id ? n : c)), e => { tell(String(e)); void load() })
  const remove = (k: Card) => ask(`Remove the card “${k.title}”?${k.routine_id ? ' Its routine makes a new one on its next run.' : ''}`, 'Remove', true)
    .then(ok => { if (ok) void invoke('card_delete', { chatId, id: k.id }).then(load, e => tell(`Couldn't remove it: ${e}`)) })
  return (
    <div className="pinned-cards">
      <GoalCard chatId={chatId} botName={botName} />
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
