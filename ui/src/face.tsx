// Each bot is a small creature in its pigment. It breathes, blinks and glances around; the one you're
// talking to follows your pointer; and it shows what it's doing: listening while you type, thinking,
// searching, reading, writing, waiting for your OK, pleased, upset. Its shape, eyes and one small
// detail come from its id, so every bot looks like itself. CSS does the moving; JS only aims the eyes.
import { useEffect, useRef, type CSSProperties } from 'react'
import { faceTiming, traits, watch, type Mood } from './face-core'
import type { Look } from './format'

export { moodOf, type Mood } from './face-core'

/**
 * The face inside an avatar. `alive`: follows the pointer (the bot you're talking to). `still`: only
 * blinks (avatars in history, so a long chat doesn't fidget). `beat` changes on every keystroke while
 * listening, for a small nod.
 */
export function Face({ id, mood = 'idle', alive, still, beat, look }: { id: number; mood?: Mood; alive?: boolean; still?: boolean; beat?: number; look?: Look }) {
  const ref = useRef<HTMLSpanElement>(null)
  useEffect(() => (alive && ref.current ? watch(ref.current) : undefined), [alive])
  const t = traits(id, look)
  return (
    <span ref={ref} className="face" aria-hidden="true" style={faceTiming(t.h) as CSSProperties} data-mood={mood} data-still={still || undefined}
      data-shape={t.shape} data-eyes={t.eyes} data-extra={t.extra || undefined} data-beat={beat === undefined ? undefined : beat % 2}>
      <span className="face-body">
        <span className="face-look"><span className="face-eyes"><i /><i /></span><span className="face-mouth" /></span>
      </span>
    </span>
  )
}
