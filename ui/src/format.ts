// Plain helpers: bot pigments and human-friendly time/schedule text.

// ---------------- bots: a pigment each ----------------

/** Muted pigments that hold up in light and dark. A bot keeps its colour (by id) for life, unless you pick one. */
export const PIGMENTS = ['#C2614A', '#4A6FC4', '#5E8F45', '#8160BE', '#BD8A2B', '#2E8F88', '#B5527D', '#5F6670', '#A86A3D', '#3D8DB5', '#8A8F3A', '#9C4F9E']

/** How a bot's creature looks, if you chose: any part left out comes from its id. */
export type Look = { shape?: string; eyes?: string; extra?: string; color?: string }
const looks = new Map<number, Look>()
/** The bots' chosen looks (from the bots list), so every avatar of a bot shows the same creature. */
export function setLooks(bots: { id: number; look?: string }[]) {
  looks.clear()
  for (const b of bots) {
    try { if (b.look) looks.set(b.id, JSON.parse(b.look)) } catch { /* an unreadable look: the default one */ }
  }
}
export const lookOf = (id: number): Look | undefined => looks.get(id)
export const pigment = (id: number, look = looks.get(id)) => look?.color ?? PIGMENTS[(Math.max(id, 1) - 1) % 8]

// ---------------- time & schedule text ----------------

// Formatters are made once: toLocale*String builds a new one on every call, which is most of a long chat's render.
const fmt = (o: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(undefined, o)
const HM = fmt({ hour: 'numeric', minute: '2-digit' }), WEEKDAY = fmt({ weekday: 'long' }), WD = fmt({ weekday: 'short' })
const LONG = fmt({ weekday: 'long', day: 'numeric', month: 'long' }), LONG_Y = fmt({ weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
const SHORT = fmt({ weekday: 'short', day: 'numeric', month: 'short' }), FULL_TIME = fmt({ dateStyle: 'medium', timeStyle: 'short' })

export const time = (d: Date) => HM.format(d)
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString()

/** "just now", "12 min ago", "3 h ago", "Yesterday 14:05", "Mon 9 Oct" */
/** "Today", "Yesterday", "Monday", then "Monday 28 September". */
export function dayLabel(ts: number) {
  const d = new Date(ts * 1000), now = new Date()
  const days = Math.round((new Date(now.toDateString()).getTime() - new Date(d.toDateString()).getTime()) / 86400000)
  if (days === 0) return 'Today'
  if (days === 1) return 'Yesterday'
  if (days < 7) return WEEKDAY.format(d)
  return (d.getFullYear() === now.getFullYear() ? LONG : LONG_Y).format(d)
}

export const fullTime = (ts: number) => FULL_TIME.format(new Date(ts * 1000))

/** A routine's end: "Thu 30 Oct" when it's through that day (23:59), else the date and time. */
export function untilText(ts: number) {
  const d = new Date(ts * 1000)
  return d.getHours() === 23 && d.getMinutes() === 59 ? SHORT.format(d) : FULL_TIME.format(d)
}

export function ago(ts: number) {
  const s = Date.now() / 1000 - ts
  const d = new Date(ts * 1000)
  if (s < 45) return 'just now'
  if (s < 3600) return `${Math.round(s / 60)} min ago`
  if (s < 6 * 3600) return `${Math.round(s / 3600)} h ago`
  const now = new Date()
  if (sameDay(d, now)) return `Today ${time(d)}`
  const y = new Date(now); y.setDate(now.getDate() - 1)
  if (sameDay(d, y)) return `Yesterday ${time(d)}`
  return SHORT.format(d)
}

/** "in 20 min", "today 19:00", "tomorrow 08:30", "Mon 08:30" (reads after "Next run") */
export function ahead(ts: number) {
  const s = ts - Date.now() / 1000
  const d = new Date(ts * 1000)
  if (s < 60) return 'in a moment'
  if (s < 3600) return `in ${Math.round(s / 60)} min`
  const now = new Date()
  if (sameDay(d, now)) return `today ${time(d)}`
  const t = new Date(now); t.setDate(now.getDate() + 1)
  if (sameDay(d, t)) return `tomorrow ${time(d)}`
  return `${WD.format(d)} ${time(d)}`
}

const DAY: Record<string, string> = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun' }
export const dayName = (d: string) => DAY[d] ?? d.slice(0, 1).toUpperCase() + d.slice(1, 3).toLowerCase()
const FULL: Record<string, string> = { Mon: 'Monday', Tue: 'Tuesday', Wed: 'Wednesday', Thu: 'Thursday', Fri: 'Friday', Sat: 'Saturday', Sun: 'Sunday' }
export const fullDay = (d: string) => FULL[dayName(d)] ?? d

/** A calendar routine's next_fire while no matching event is coming up (db::IDLE). */
export const IDLE = 4_102_444_800

/** Cron → plain words for the common shapes; anything else is shown as-is. */
export function schedule(cron: string): string {
  if (cron.trim() === 'webhook') return 'When its webhook is called'
  if (cron.trim().startsWith('mail:')) return `When an email matches ${cron.trim().slice(5).trim()}`
  const cal = cron.trim().match(/^calendar:(\d+)(?::(meetings|outside))?$/)
  if (cal) {
    const m = +cal[1], lead = m % 60 === 0 && m > 0 ? `${m / 60} hour${m === 60 ? '' : 's'}` : `${m} min`
    return `${lead} before each ${cal[2] === 'meetings' ? 'meeting with someone else' : cal[2] === 'outside' ? 'meeting with someone outside' : 'calendar event'}`
  }
  const f = cron.trim().split(/\s+/)
  if (f.length !== 5) return cron
  const [m, h, dom, mon, dow] = f
  const num = (x: string) => /^\d+$/.test(x)
  const every = (x: string) => x.match(/^\*\/(\d+)$/)?.[1]
  if (num(m) && num(h) && mon === '*') {
    const at = time(new Date(2000, 0, 1, +h, +m))
    if (dom === '*') {
      const d = dow.toLowerCase()
      if (d === '*') return `Every day at ${at}`
      if (d === 'mon-fri' || d === '1-5') return `Weekdays at ${at}`
      if (['sat,sun', 'sun,sat', 'sat-sun', '0,6', '6,0'].includes(d)) return `Weekends at ${at}`
      if (/^\w+$/.test(dow) && FULL[dayName(dow)]) return `Every ${fullDay(dow)} at ${at}`
      return `${dow.split(',').map(p => p.split('-').map(dayName).join('–')).join(', ')} at ${at}`
    }
    if (num(dom) && dow === '*') return `Monthly on day ${dom} at ${at}`
  }
  if (m === '0' && every(h) && dom === '*' && mon === '*' && dow === '*') return `Every ${every(h)} hours`
  if (m === '0' && h === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every hour'
  if (every(m) && h === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${every(m)} minutes`
  return cron
}

export const tok = (n: number) => n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n)
export const usd = (n: number) => n < 0.01 && n > 0 ? '<$0.01' : `$${n.toFixed(2)}`
