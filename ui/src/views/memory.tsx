import { useContext, useEffect, useState } from 'react'
import { invoke } from '../api'
import { ago } from '../format'
import { Markdown } from '../markdown'
import { ask, tell, Empty, Icon, Segmented } from '../ui'
import { useLeave } from './inbox'
import { useLoad, HOUSEHOLD, Bots } from '../shared'
import type { Kind, Skill } from '../types'

type EntryRow = {
  id: number; kind: Kind; title: string; body: string; source: string; created_at: number
  status: 'active' | 'superseded' | 'forgotten'; superseded_by: number | null; access_count: number; pinned?: boolean; background?: boolean
  origin?: '' | 'you' | 'bot' | 'tidy' | 'import' | 'know_me'; bot_id?: number | null; tainted?: boolean; missteps?: number; valid_until?: number | null; asked?: string | null
  /** Kept by one bot only (0: all bots). */
  for_bot?: number
}

type Suggestion = { id: number; action: 'add' | 'update' | 'forget' | 'demote'; entry_id: number | null; type: Kind; title: string; body: string; reason: string; check?: string | null; tainted?: boolean }

const KINDS: { id: Kind; title: string; empty: string }[] = [
  { id: 'convention', title: 'Preferences', empty: 'How you like things done.' },
  { id: 'knowledge', title: 'Facts', empty: 'People, places and details about your life.' },
  { id: 'task', title: 'Tasks', empty: 'Things you asked it to keep track of.' },
]

const KIND_SINGULAR: Record<Kind, string> = { convention: 'Preference', knowledge: 'Fact', task: 'Task' }

export function MemoryView({ chatId, onChange }: { chatId: number; onChange: () => void }) {
  const rows = useLoad(() => invoke<EntryRow[]>('entries', { chatId }), [chatId])
  // How much of the standing memory (what every reply is given) is used.
  const standing = useLoad(() => invoke<{ used: number; budget: number; count: number }>('standing_memory', { chatId }), [chatId, rows.data?.map(r => `${r.id}:${r.status}:${r.pinned}:${r.background}`).join()])
  const [query, setQuery] = useState('')
  const [found, setFound] = useState<number[] | null>(null)
  const [history, setHistory] = useState<'now' | 'all' | 'timeline'>('now')
  const [adding, setAdding] = useState<'one' | 'import' | null>(null)
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => {
    const q = query.trim()
    if (!q) return
    const t = setTimeout(() => invoke<{ id: number }[]>('search', { chatId, query: q }).then(r => setFound(r.map(e => e.id)), e => setErr(String(e))), 150)
    return () => clearTimeout(t)
  }, [query, chatId])

  const hits = query.trim() ? found : null
  const refresh = () => { rows.reload(); onChange() }
  const act = (p: Promise<unknown>) => p.then(() => { setErr(null); refresh() }, e => setErr(String(e)))

  const all = rows.data ?? []
  const byId = new Map(all.map(r => [r.id, r]))
  const inactive = all.filter(r => r.status !== 'active').length
  const visible = history === 'all' ? all : all.filter(r => r.status === 'active')
  // id → the entry it replaced, to show lineage on the successor.
  const replaced = new Map(all.filter(r => r.superseded_by).map(r => [r.superseded_by as number, r.id]))
  // Earlier versions of a memory, newest first.
  const earlier = (id: number) => {
    const out: EntryRow[] = []
    for (let p = replaced.get(id); p !== undefined && out.length < 50; p = replaced.get(p)) { const r = byId.get(p); if (r) out.push(r) }
    return out
  }
  const card = (r: EntryRow, showKind = false) => <MemoryCard key={r.id} r={r} versions={earlier(r.id)} act={act} chatId={chatId} showKind={showKind} />
  const pinned = history === 'now' ? visible.filter(r => r.pinned) : []
  const background = history === 'now' ? visible.filter(r => r.background) : []

  return (
    <>
      <div className="searchbar">
        <label className="search"><Icon name="search" size={15} /><input className="input" placeholder="Search memory" aria-label="Search memory" value={query} onChange={e => setQuery(e.target.value)} /></label>
        <Segmented size="sm" label="Which memories" value={history} onChange={setHistory}
          options={[{ id: 'now', label: 'Current' }, { id: 'all', label: `History${inactive ? ` · ${inactive}` : ''}` }, { id: 'timeline', label: 'Timeline' }]} />
        {/* The tidy-up reads a chat's conversations; the household has none. */}
        {chatId !== HOUSEHOLD && <TidyButton chatId={chatId} onDone={refresh} />}
        <button className="btn btn-ghost" onClick={() => setAdding(a => a === 'import' ? null : 'import')} title="Bring in what another assistant remembers about you">Import</button>
        <button className="btn" onClick={() => setAdding(a => a === 'one' ? null : 'one')}><Icon name="plus" size={14} />Add</button>
      </div>
      {(err || rows.error) && <p className="error">{err || rows.error}</p>}
      {standing.data && standing.data.count > 0 && (() => {
        const pct = Math.round(standing.data.used * 100 / standing.data.budget)
        return <div className="standing" title="Every reply is given your pinned memories, your preferences and your most recent facts and tasks, within this budget. The rest is found by search when a message needs it.">
          <div className="standing-bar"><i style={{ width: `${Math.min(pct, 100)}%` }} className={pct >= 90 ? 'full' : ''} /></div>
          <span className="hint">Standing memory {pct}% full · {standing.data.count} {standing.data.count === 1 ? 'memory' : 'memories'} in every reply{pct >= 90 ? '; pin less, or move some to background' : ''}</span>
        </div>
      })()}
      <Suggestions chatId={chatId} entries={all} onChange={refresh} />
      <ConfirmAll chatId={chatId} entries={all} onDone={refresh} />
      <Learned chatId={chatId} entries={all} onChange={refresh} />
      <Files chatId={chatId} />
      {adding === 'one' && <div className="editor-card"><Editor submit="Save memory" onCancel={() => setAdding(null)} onSave={(kind, title, body) => act(invoke('remember', { chatId, kind, title, body, supersedes: null })).then(() => setAdding(null))} /></div>}
      {adding === 'import' && <ImportMemory chatId={chatId} onDone={() => { setAdding(null); refresh() }} />}

      {hits ? (
        <>
          <div className="section-title"><h2>Results</h2><span className="count">{hits.length}</span></div>
          {hits.length ? <div className="mem-list">{hits.map(id => byId.get(id)).filter((r): r is EntryRow => !!r).map(r => card(r, true))}</div>
            : <Empty icon="search" title="Nothing matches">Search matches whole words and the start of words.</Empty>}
        </>
      ) : history === 'timeline' ? (
        <Timeline chatId={chatId} entries={all} card={card} />
      ) : visible.length === 0 ? (
        <Empty icon="memory" title="Nothing remembered yet">
          Tell a bot something about yourself, like “my dentist is Dr. Lee”. It’s saved here with where it was learned, and you can correct or forget it anytime.
        </Empty>
      ) : [{ id: 'pinned', title: 'Pinned' }, ...KINDS, { id: 'background', title: 'Background' }].map(k => {
        const items = k.id === 'pinned' ? pinned : k.id === 'background' ? background : visible.filter(r => r.kind === k.id && !pinned.includes(r) && !background.includes(r))
        return items.length ? (
          <section key={k.id}>
            <div className="section-title"><h2>{k.title}</h2><span className="count">{items.length}</span>
              {k.id === 'background' && <span className="hint">Not kept in mind, but found when a message needs it.</span>}</div>
            <div className="mem-list">{items.map(r => card(r))}</div>
          </section>
        ) : null
      })}
    </>
  )
}

function TidyButton({ chatId, onDone }: { chatId: number; onDone: () => void }) {
  const [state, setState] = useState<string | null>(null)
  const run = () => {
    setState('Reviewing…')
    invoke<number>('tidy', { chatId }).then(
      n => { setState(n ? `${n} suggestion${n === 1 ? '' : 's'}` : 'All tidy'); onDone(); setTimeout(() => setState(null), 4000) },
      e => setState(String(e)),
    )
  }
  return (
    <button className="btn btn-ghost" onClick={run} disabled={state === 'Reviewing…'} title="Review memory against recent conversations. Runs by itself once a day when the chat is quiet.">
      <Icon name="wand" size={14} />{state ?? 'Tidy up'}
    </button>
  )
}

/** Memories saved after reading outside content wait for "Looks right": all of them at once, after you've read them. */
function ConfirmAll({ chatId, entries, onDone }: { chatId: number; entries: EntryRow[]; onDone: () => void }) {
  const waiting = entries.filter(e => e.tainted && e.status === 'active')
  if (waiting.length < 2) return null
  const confirm = async () => {
    if (!await ask(`Mark all ${waiting.length} as right? They were saved after a bot read outside content (a page, a file, an email), so check they say what you meant: your bots then keep them in mind like anything you said.`, 'All look right')) return
    for (const e of waiting) await invoke('confirm_memory', { chatId, id: e.id }).catch(err => tell(String(err)))
    onDone()
  }
  return (
    <div className="confirm-all"><Icon name="shield" size={14} /><span>{waiting.length} memories wait for you to say they look right (marked “after reading outside content” below).</span>
      <button className="btn btn-sm" onClick={confirm}>All look right</button></div>
  )
}

const ACTION: Record<Suggestion['action'], [string, string]> = { add: ['Remember', 'live'], update: ['Update', 'pending'], forget: ['Forget', 'danger'], demote: ['Background', 'pending'] }

/** The other memory of a duplicate or a contradiction ("Duplicate of #N", "Contradicts #N"): the one the card keeps. */
const pairedId = (reason: string) => { const m = /^(?:Duplicate of|Contradicts) #(\d+)/.exec(reason); return m ? Number(m[1]) : null }

/** Proposed changes from the tidy-up. Nothing changes until accepted. */
function Suggestions({ chatId, entries, onChange }: { chatId: number; entries: EntryRow[]; onChange: () => void }) {
  const list = useLoad(() => invoke<Suggestion[]>('suggestions', { chatId }), [chatId], 10000)
  const [err, setErr] = useState<string | null>(null)
  const [checking, setChecking] = useState<Set<number>>(new Set())
  const { leaving, leave } = useLeave()
  const items = list.data ?? []
  if (!items.length) return null
  const target = (id: number | null) => entries.find(e => e.id === id)
  const check = (id: number) => {
    setChecking(c => new Set(c).add(id))
    invoke('check_suggestion', { chatId, id }).then(() => list.reload(), e => setErr(String(e)))
      .finally(() => setChecking(c => { const n = new Set(c); n.delete(id); return n }))
  }
  const decide = (id: number, accept: boolean) =>
    leave(id, () => invoke('decide_suggestion', { chatId, id, accept }).then(() => { setErr(null); return list.reload().then(onChange) }, e => setErr(String(e))))
  const forgetFor = (id: number, entryId: number) =>
    leave(id, () => invoke('forget_for_suggestion', { chatId, id, entryId }).then(() => { setErr(null); return list.reload().then(onChange) }, e => setErr(String(e))))
  // Which side of a contradiction is right is yours to say: Accept all leaves them.
  const clash = (s: Suggestion) => s.reason.startsWith('Contradicts #')
  const all = async (accept: boolean) => {
    if (!accept && !await ask(`Dismiss all ${items.length} suggestions?`, 'Dismiss all', true)) return
    for (const s of items.filter(s => !accept || !clash(s))) await invoke('decide_suggestion', { chatId, id: s.id, accept }).catch(e => setErr(String(e)))
    list.reload().then(onChange)
  }
  return (
    <section className="suggest" aria-label="Suggested memory changes">
      <div className="suggest-head"><Icon name="wand" /><b>{items.length} suggested {items.length === 1 ? 'change' : 'changes'}</b><small>{items.every(s => s.reason.startsWith('Imported')) ? 'From what you imported.' : 'From recent conversations.'} Nothing changes until you accept.</small>
        {/* All at once. Ones drawn from outside content stay marked unconfirmed after accepting, as one by one. */}
        {items.length > 1 && <span className="suggest-all">
          <button className="btn btn-sm btn-ghost" onClick={() => all(false)}>Dismiss all</button>
          <button className="btn btn-sm" onClick={() => all(true)} title={[items.some(s => s.tainted) && 'The ones drawn from outside content stay marked unconfirmed until you say they look right.', items.some(clash) && 'Contradictions wait for you to pick which to keep.'].filter(Boolean).join(' ') || undefined}>Accept all</button>
        </span>}</div>
      {items.map(s => {
        const t = target(s.entry_id)
        const [label, tone] = ACTION[s.action]
        const other = s.action === 'forget' ? target(pairedId(s.reason)) : undefined
        // A pair: both memories side by side, and you keep either one (or both).
        if (other) return (
          <div className={'sugg' + (leaving.has(s.id) ? ' leaving' : '')} key={s.id}>
            <span className={'badge ' + (clash(s) ? 'danger' : 'pending')}>{clash(s) ? 'Conflict' : 'Duplicate'}</span>
            <div className="what selectable">
              {[{ e: t, keep: () => forgetFor(s.id, other.id) }, { e: other, keep: () => decide(s.id, true) }].map(({ e, keep }, i) => (
                <div className="either" key={i}><span><b>{e?.title ?? s.title}</b>{e?.body ?? s.body}</span>
                  <button className="btn btn-sm btn-ghost" onClick={keep}>Keep this one</button></div>
              ))}
              <div className="why">{s.reason.replace(/^(?:Duplicate of|Contradicts) #\d+ \("[^"]*"\)\.?\s*/, '') || 'They say the same thing.'} The other one is forgotten (it stays in History).</div>
            </div>
            <div className="choices"><button className="btn btn-sm btn-ghost" onClick={() => decide(s.id, false)}>Keep both</button></div>
          </div>
        )
        return (
          <div className={'sugg ' + s.action + (leaving.has(s.id) ? ' leaving' : '')} key={s.id}>
            <span className={'badge ' + tone}>{label}</span>
            <div className="what selectable">
              {s.action === 'forget' || s.action === 'demote'
                ? <><b>{t?.title ?? s.title}</b><span className="old">{t?.body ?? s.body}</span></>
                : <><b>{s.title}</b>{s.body}{s.action === 'update' && t && <div className="old">Was: {t.body}</div>}</>}
              <div className="why">{s.reason}</div>
              {s.tainted && <div className="why warn-line"><Icon name="shield" size={11} />Drawn from outside content (a page, a file, a forwarded message), not your own words. Accepting keeps it marked unconfirmed.</div>}
              {s.check && <div className="why checked"><Icon name="history" size={11} />{s.check}</div>}
              {checking.has(s.id) && <div className="why"><span className="typing" aria-hidden="true"><i /><i /><i /></span>Replaying past corrections…</div>}
            </div>
            <div className="choices">
              {(s.action === 'add' || s.action === 'update') && !s.check && !checking.has(s.id) && (
                <button className="btn btn-sm btn-ghost" onClick={() => check(s.id)} title="Re-answer past corrections with this change and see if it would have helped. Uses your AI account for a few short replies.">Check</button>
              )}
              {/* Unused for months: keep it, move it out of mind, or let it go. */}
              {s.action === 'demote' && s.entry_id !== null && <button className="btn btn-sm btn-ghost btn-danger" onClick={() => forgetFor(s.id, s.entry_id!)}>Forget</button>}
              <button className="btn btn-sm btn-ghost" onClick={() => decide(s.id, false)}>{s.action === 'demote' ? 'Keep' : 'Dismiss'}</button>
              <button className="btn btn-sm" onClick={() => decide(s.id, true)}>{s.action === 'demote' ? 'Background' : 'Accept'}</button>
            </div>
          </div>
        )
      })}
      {err && <p className="error" style={{ padding: '0 14px' }}>{err}</p>}
    </section>
  )
}

type Week = { start: number; turns: number; corrections: number; stops: number; denials: number; tool_calls: number; tool_errors: number; accepted: number; dismissed: number; repeats: number; forgot: number; undone: number }

/** This week against the three before it, as short phrases; only what there's data for. */
function trends(weeks: Week[]): string[] {
  const now = weeks[weeks.length - 1], before = weeks.slice(0, -1)
  if (!now) return []
  const sum = (ws: Week[], k: keyof Week) => ws.reduce((n, w) => n + w[k], 0)
  const per10 = (ws: Week[], k: keyof Week) => sum(ws, 'turns') ? Math.round(sum(ws, k) * 100 / sum(ws, 'turns')) / 10 : null
  const out: string[] = []
  const was = (v: number | null) => v === null ? '' : ` (was ${v})`
  if (now.turns) out.push(`${per10([now], 'corrections')} corrections per 10 messages${was(per10(before, 'corrections'))}`)
  if (now.repeats) out.push(`${now.repeats} repeated an earlier correction`)
  if (now.forgot) out.push(`${now.forgot} “I told you”`)
  if (now.denials || now.stops) out.push(`${now.denials} denied, ${now.stops} stopped`)
  if (now.tool_calls) out.push(`${Math.round(now.tool_errors * 100 / now.tool_calls)}% of actions failed`)
  if (now.accepted + now.dismissed) out.push(`${now.accepted} of ${now.accepted + now.dismissed} suggestions accepted`)
  if (now.undone) out.push(`${now.undone} learned ${now.undone === 1 ? 'memory' : 'memories'} taken back`)
  return out
}

/** The weekly look back: what was learned without you typing it, since you last checked (each can be undone),
 *  and how this week went compared with the ones before. */
function Learned({ chatId, entries, onChange }: { chatId: number; entries: EntryRow[]; onChange: () => void }) {
  // The clock is read when loading, not while rendering.
  const info = useLoad(() => invoke<{ weeks: Week[]; seen: number }>('learning', { chatId }).then(d => ({ ...d, now: Date.now() / 1000 })), [chatId], 60000)
  const [err, setErr] = useState<string | null>(null)
  const { leaving, leave } = useLeave()
  const botOf = useContext(Bots)
  if (!info.data) return null
  const since = Math.max(info.data.seen, info.data.now - 7 * 86400)
  const items = entries.filter(r => r.status === 'active' && r.origin && r.origin !== 'you' && r.created_at > since)
  if (!items.length) return null
  const undo = (id: number) => leave(id, () => invoke('undo_memory', { chatId, id }).then(() => { setErr(null); onChange() }, e => setErr(String(e))))
  const replaced = new Set(entries.map(e => e.superseded_by))
  const trend = trends(info.data.weeks)
  return (
    <section className="suggest weekly" aria-label="Learned this week">
      <div className="suggest-head"><Icon name="memory" /><b>Learned this week</b><small>Saved without you typing it. Undo anything that's wrong.</small>
        <button className="btn btn-sm btn-ghost" onClick={() => invoke('learned_seen', { chatId }).then(() => info.reload(), e => tell(String(e)))}>All good</button></div>
      {items.map(r => (
        <div className={'sugg' + (leaving.has(r.id) ? ' leaving' : '')} key={r.id}>
          <span className="badge">{r.origin === 'tidy' ? 'Tidy-up' : r.origin === 'import' ? 'Imported' : r.origin === 'know_me' ? 'Email & calendar' : r.bot_id ? botOf(r.bot_id).name : 'A bot'}</span>
          <div className="what selectable"><b>{r.title}</b>{r.body}
            {/* Their own words next to what was saved: a remark saved as firmer than they said it shows. */}
            {r.asked && <div className="why">You said: “{r.asked.length > 160 ? r.asked.slice(0, 160) + '…' : r.asked}”</div>}
            <div className="why">{replaced.has(r.id) ? 'Replaced an earlier version; undo brings it back. ' : ''}{ago(r.created_at)}</div></div>
          <div className="choices"><button className="btn btn-sm btn-ghost" onClick={() => undo(r.id)}>Undo</button></div>
        </div>
      ))}
      {trend.length > 0 && <p className="weekly-trend">This week: {trend.join(' · ')}</p>}
      {err && <p className="error" style={{ padding: '0 14px' }}>{err}</p>}
    </section>
  )
}

/** What your bots learned, by day: memories saved without you typing them and the skills you accepted, newest first. */
function Timeline({ chatId, entries, card }: { chatId: number; entries: EntryRow[]; card: (r: EntryRow, showKind?: boolean) => React.ReactNode }) {
  const skills = useLoad(() => invoke<Skill[]>('skills', { chatId }), [chatId])
  const [err, setErr] = useState<string | null>(null)
  const botOf = useContext(Bots)
  // Library skills are ones you added, not learned.
  const items = [
    ...entries.filter(r => r.status === 'active' && r.origin && r.origin !== 'you').map(r => ({ at: r.created_at, node: card(r, true) })),
    ...(skills.data ?? []).filter(k => k.status === 'active' && k.kind !== 'library').map(k => ({
      at: k.created_at, node: (
        <article className="mem" key={`s${k.id}`}>
          <div className="mem-actions">
            <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Remove" aria-label={`Remove ${k.name}`}
              onClick={() => invoke('remove_skill', { chatId, id: k.id }).then(() => { setErr(null); return skills.reload() }, e => setErr(String(e)))}><Icon name="trash" size={14} /></button>
          </div>
          <div className="mem-title"><Icon name="bolt" size={12} />{k.name}</div>
          <p className="mem-body">{k.description}</p>
          <div className="mem-meta"><span>Skill</span><span>{k.bot_id ? `for ${botOf(k.bot_id).name}` : 'for all bots'}</span><span>{ago(k.created_at)}</span></div>
        </article>
      ),
    })),
  ].sort((a, b) => b.at - a.at)
  if (!items.length) return <Empty icon="memory" title="Nothing learned yet">What your bots pick up from conversations, and skills you accept, show here by the day they were learned.</Empty>
  const days = new Map<string, typeof items>()
  for (const i of items) {
    const d = new Date(i.at * 1000).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' })
    days.set(d, [...(days.get(d) ?? []), i])
  }
  return (
    <>
      {err && <p className="error">{err}</p>}
      {[...days].map(([d, list]) => (
        <section key={d}>
          <div className="section-title"><h2>{d}</h2><span className="count">{list.length}</span></div>
          <div className="mem-list">{list.map(i => i.node)}</div>
        </section>
      ))}
    </>
  )
}

/** The chat's file workspace: what the bots wrote there (notes, lists, drafts), to read or delete. */
function Files({ chatId }: { chatId: number }) {
  const list = useLoad(() => invoke<{ path: string; size: number }[]>('files', { chatId }), [chatId], 15000)
  const [open, setOpen] = useState<{ path: string; text: string } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  if (!list.data?.length) return null
  const show = (path: string) => open?.path === path ? setOpen(null) : invoke<string>('file', { chatId, path }).then(text => { setErr(null); setOpen({ path, text }) }, e => setErr(String(e)))
  const remove = (path: string) => ask(`Delete ${path}?`, 'Delete', true).then(ok => { if (ok) void invoke('delete_file', { chatId, path }).then(() => { if (open?.path === path) setOpen(null); return list.reload() }, e => setErr(String(e))) })
  return (
    <section aria-label="Files">
      <div className="section-title"><h2>Files</h2><span className="count">{list.data.length}</span></div>
      <p className="hint">Your bots' workspace in this chat: notes, lists and drafts too long for a memory. A rewritten file keeps its earlier version.</p>
      {err && <p className="error">{err}</p>}
      <div className="mem-list">{list.data.map(f => (
        <article className="mem" key={f.path}>
          <div className="mem-actions">
            <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Delete" aria-label={`Delete ${f.path}`} onClick={() => remove(f.path)}><Icon name="trash" size={14} /></button>
          </div>
          <div className="mem-title"><Icon name="file" size={13} /> {f.path}</div>
          {open?.path === f.path && <div className="skill-steps selectable">{f.path.endsWith('.md') ? <Markdown text={open.text} /> : <pre className="file-text">{open.text}</pre>}</div>}
          <div className="mem-meta">
            <button type="button" className="linkish" aria-expanded={open?.path === f.path} onClick={() => show(f.path)}>{open?.path === f.path ? 'hide' : 'open'}</button>
            <span>{f.size < 1024 ? `${f.size} bytes` : `${Math.round(f.size / 1024)} KB`}</span>
          </div>
        </article>
      ))}</div>
    </section>
  )
}

type AgentSource = { id: string; label: string; memories: number; skills: number; routines: number }

/** Paste another assistant's memories; the memory bot turns them into suggestions to accept or dismiss. */
function ImportMemory({ chatId, onDone }: { chatId: number; onDone: () => void }) {
  const [text, setText] = useState('')
  const [state, setState] = useState<{ busy?: boolean; err?: string; note?: string }>({})
  const go = () => {
    setState({ busy: true })
    invoke<number>('import_memory', { chatId, text }).then(
      n => n ? onDone() : setState({ note: 'Nothing new to add: your memory already has all of it.' }),
      e => setState({ err: String(e) }))
  }
  // Other agents on this computer, brought in with one click: memories as suggestions, skills as proposals, jobs as paused routines.
  const agents = useLoad(() => invoke<AgentSource[]>('import_sources'), []).data
  const many = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`
  const bring = (a: AgentSource) => {
    setState({ busy: true })
    invoke<{ memories: number; skills: number; routines: number }>('import_agent', { chatId, source: a.id }).then(
      r => {
        const got = [r.memories && `${many(r.memories, 'suggested memory')} below`, r.skills && `${many(r.skills, 'skill')} to review in Skills`, r.routines && `${many(r.routines, 'routine')} paused in Routines (resume one to start it)`].filter(Boolean)
        setState({ note: got.length ? `From ${a.label}: ${got.join(', ')}.` : `Nothing new from ${a.label}: you already have all of it.` })
        if (r.memories) onDone()
      },
      e => setState({ err: String(e) }))
  }
  return (
    <div className="editor-card import">
      {!!agents?.length && <div className="er">
        <span className="grow hint">Other agents found where Mimir runs:</span>
        {agents.map(a => <button key={a.id} className="btn btn-ghost" disabled={state.busy} onClick={() => bring(a)}
          title={[many(a.memories, 'memory file'), many(a.skills, 'skill'), many(a.routines, 'scheduled job')].join(', ')}>From {a.label}</button>)}
      </div>}
      <p className="hint" style={{ margin: 0 }}>Paste what another assistant remembers about you. In ChatGPT: Settings → Personalization → Manage memories; or ask any assistant “List everything you remember about me.”</p>
      <textarea className="textarea" rows={7} aria-label="Memories to import" placeholder={'- Lives in Cape Town\n- Prefers short answers\n- Training for a half marathon in March'} value={text} onChange={e => setText(e.target.value)} autoFocus />
      <div className="er">
        <span className="grow">{state.err ? <span className="error">{state.err}</span> : state.note ? <span className="hint">{state.note}</span> : <span className="hint">You'll see each one as a suggestion first. Nothing is saved until you accept it.</span>}</span>
        {state.busy && <span className="typing" aria-hidden="true"><i /><i /><i /></span>}
        <button className="btn btn-primary" disabled={!text.trim() || state.busy} onClick={go}>{state.busy ? 'Reading…' : 'Suggest memories'}</button>
      </div>
    </div>
  )
}

function MemoryCard({ r, versions, act, chatId, showKind }: { r: EntryRow; versions: EntryRow[]; act: (p: Promise<unknown>) => Promise<unknown>; chatId: number; showKind: boolean }) {
  const [editing, setEditing] = useState(false)
  const [showVersions, setShowVersions] = useState(false)
  const [check, setCheck] = useState<{ busy?: boolean; note?: string }>({})
  const botOf = useContext(Bots)
  // Who wrote it is recorded by Mimir; "source" is what the bot said about where it learned it.
  const who = r.origin === 'you' ? 'added by you' : r.origin === 'tidy' ? 'from a tidy-up' : r.origin === 'import' ? 'imported'
    : r.origin === 'bot' && !r.bot_id ? 'suggested by a bot, approved by you'
    : r.origin === 'bot' ? `learned by ${r.bot_id ? botOf(r.bot_id).name : 'a bot'}${r.source ? ` · “${r.source}”` : ''}` : r.source || 'no source'
  return (
    <article className={'mem ' + r.status + (r.pinned ? ' pinned' : '')}>
      {r.status !== 'active' && (
        <div className="mem-actions">
          <button className="btn btn-ghost btn-danger btn-sm" title="Delete it for good, with its earlier versions" onClick={() => {
            ask(`Erase “${r.title}” for good${versions.length ? `, with ${versions.length} earlier version${versions.length === 1 ? '' : 's'}` : ''}? It can't be undone.`, 'Erase', true).then(ok => { if (ok) act(invoke('erase', { chatId, id: r.id })) })
          }}>Erase</button>
        </div>
      )}
      {r.status === 'active' && !editing && (
        <div className="mem-actions">
          <button className={'btn btn-ghost btn-sm icon-btn' + (r.pinned ? ' on' : '')} aria-pressed={!!r.pinned} title={r.pinned ? 'Unpin' : 'Pin: your bots always keep this in mind'} aria-label={`${r.pinned ? 'Unpin' : 'Pin'} ${r.title}`}
            onClick={() => act(invoke('pin', { chatId, id: r.id, pinned: !r.pinned }))}><Icon name="pin" size={14} /></button>
          <button className={'btn btn-ghost btn-sm icon-btn' + (r.background ? ' on' : '')} aria-pressed={!!r.background}
            title={r.background ? 'Bring back: your bots keep this in mind again' : 'Move to background: out of mind, but found when a message needs it'}
            aria-label={`${r.background ? 'Bring back' : 'Move to background'} ${r.title}`}
            onClick={() => act(invoke('background', { chatId, id: r.id, on: !r.background }))}><Icon name="archive" size={14} /></button>
          <button className="btn btn-ghost btn-sm icon-btn" title="Correct" aria-label={`Correct ${r.title}`} onClick={() => setEditing(true)}><Icon name="pencil" size={14} /></button>
          <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Forget" aria-label={`Forget ${r.title}`} onClick={() => act(invoke('forget', { chatId, id: r.id }))}><Icon name="trash" size={14} /></button>
        </div>
      )}
      <div className="mem-title">{r.pinned && <Icon name="pin" size={12} />}{r.title}</div>
      {editing ? (
        <Editor initial={r} submit="Save correction" onCancel={() => setEditing(false)}
          onSave={(kind, title, body) => act(invoke('remember', { chatId, kind, title, body, supersedes: r.id })).then(() => setEditing(false))} />
      ) : (
        <p className="mem-body">{r.body}</p>
      )}
      <div className="mem-meta">
        {showKind && <span>{KIND_SINGULAR[r.kind]}</span>}
        <span>{who}</span>
        {r.tainted && <span className="warn" title="A bot saved this in a conversation that had read outside content (a web page, a file, a tool’s result). Check it says what you meant; until you confirm it, your bots don't keep it in mind (they only see it, labelled, if they search for it).">after reading outside content</span>}
        {r.tainted && r.status === 'active' && <button type="button" className="linkish" onClick={() => act(invoke('confirm_memory', { chatId, id: r.id }))}>Looks right</button>}
        <span>{ago(r.created_at)}</span>
        {/* Kept by one bot only, or by all: the bot that keeps it (or learned it) can have it to itself. */}
        {r.status === 'active' && chatId !== HOUSEHOLD && (r.for_bot || (r.origin === 'bot' && r.bot_id)) ? (
          <select className="inline-date" aria-label={`Which bots keep ${r.title}`} value={r.for_bot ?? 0}
            title="Only one bot keeps this in mind (your other bots don't see it), or all of them."
            onChange={e => act(invoke('memory_for_bot', { chatId, id: r.id, botId: Number(e.target.value) }))}>
            <option value={0}>all bots</option>
            <option value={r.for_bot || r.bot_id!}>only {botOf(r.for_bot || r.bot_id!).name}</option>
          </select>
        ) : null}
        {r.valid_until && r.status === 'active' && (
          <span className={r.valid_until * 1000 <= Date.now() ? 'gone' : undefined}
            title="Your bots keep it in mind through this day; after that only search finds it, marked as ended.">
            {r.valid_until * 1000 <= Date.now() ? 'ended ' : 'until '}
            {/* sv-SE formats a local date as YYYY-MM-DD, what a date input takes. */}
            <input type="date" className="inline-date" aria-label={`End date of ${r.title}`} value={new Date(r.valid_until * 1000).toLocaleDateString('sv-SE')}
              onChange={e => e.target.value && act(invoke('set_until', { chatId, id: r.id, until: e.target.value }))} />
            {' '}<button type="button" className="linkish" onClick={() => act(invoke('set_until', { chatId, id: r.id, until: null }))}>no end</button>
          </span>
        )}
        {r.access_count > 0 && <span>recalled {r.access_count}×</span>}
        {r.kind === 'convention' && r.status === 'active' && !check.busy && !check.note && (
          <button type="button" className="linkish" title="Re-answer your past corrections about this with and without it, on your bot's current model (useful after switching models). Uses your AI account for a few short replies."
            onClick={() => { setCheck({ busy: true }); invoke<string>('check_memory', { chatId, id: r.id }).then(note => setCheck({ note }), e => setCheck({ note: String(e) })) }}>Check</button>
        )}
        {(r.missteps ?? 0) >= 2 && r.status === 'active' && <span className="warn" title="Replies that used this memory were corrected. If it's wrong or outdated, correct it; the tidy-up will also look at it.">corrected after use {r.missteps}×</span>}
        {versions.length > 0 && <button type="button" className="lineage linkish" aria-expanded={showVersions} onClick={() => setShowVersions(v => !v)}>
          {showVersions ? 'hide' : versions.length === 1 ? 'earlier version' : `${versions.length} earlier versions`}</button>}
        {r.status === 'superseded' && <span className="lineage">replaced by #{r.superseded_by}</span>}
        {r.status === 'forgotten' && <span className="gone">forgotten</span>}
        <span>#{r.id}</span>
      </div>
      {check.busy && <p className="mem-check"><span className="typing" aria-hidden="true"><i /><i /><i /></span>Replaying past corrections…</p>}
      {check.note && (
        <p className="mem-check">{check.note}
          {/* Fixes nothing the model doesn't already get right: it can leave standing memory. */}
          {check.note.startsWith('Fixes 0 ') && check.note.includes('already fine') && !r.background && <>{' '}
            <button type="button" className="linkish" onClick={() => act(invoke('background', { chatId, id: r.id, on: true }))}>Move to background</button></>}
        </p>
      )}
      {showVersions && (
        <ol className="versions">
          {versions.map(v => <li key={v.id}><span className="selectable">{v.title !== r.title && <b>{v.title}: </b>}{v.body}</span><small>{ago(v.created_at)} · #{v.id}</small></li>)}
        </ol>
      )}
    </article>
  )
}

export function Editor({ initial, submit, onSave, onCancel }: {
  initial?: { kind: Kind; title: string; body: string }; submit: string
  onSave: (kind: Kind, title: string, body: string) => void; onCancel?: () => void
}) {
  const [kind, setKind] = useState<Kind>(initial?.kind ?? 'knowledge')
  const [title, setTitle] = useState(initial?.title ?? '')
  const [body, setBody] = useState(initial?.body ?? '')
  const ok = title.trim() && body.trim()
  return (
    <form className="editor" onSubmit={e => { e.preventDefault(); if (ok) onSave(kind, title.trim(), body.trim()) }}>
      <div className="er">
        <select className="select" value={kind} onChange={e => setKind(e.target.value as Kind)} aria-label="Type">
          <option value="convention">Preference</option>
          <option value="knowledge">Fact</option>
          <option value="task">Task</option>
        </select>
        <input className="input" placeholder="Short label, e.g. Dentist" value={title} onChange={e => setTitle(e.target.value)} autoFocus />
      </div>
      <textarea className="textarea" placeholder="The fact itself, e.g. My dentist is Dr. Naledi Lee in Stellenbosch." value={body} onChange={e => setBody(e.target.value)} />
      <div className="er" style={{ justifyContent: 'flex-end' }}>
        {onCancel && <button type="button" className="btn btn-ghost" onClick={onCancel}>Cancel</button>}
        <button className="btn btn-primary" disabled={!ok}>{submit}</button>
      </div>
    </form>
  )
}
