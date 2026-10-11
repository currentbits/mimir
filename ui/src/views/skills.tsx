import { useContext, useState } from 'react'
import { copyText, invoke } from '../api'
import { ago } from '../format'
import { Markdown } from '../markdown'
import { Icon } from '../ui'
import { useLeave } from './inbox'
import { useLoad, Bots, RehearsalResult } from '../shared'
import type { LibrarySkill, Bot, Skill, Rehearsed } from '../types'

/** A fix as changed lines: what goes (−) and what comes (+), in order. */
/** The lines that change, in order (a longest-common-subsequence diff), so moving or repeating a step shows too. */
const lineChanges = (before: string, after: string) => {
  const a = before.split('\n'), b = after.split('\n')
  const lcs = a.map(() => new Array<number>(b.length + 1).fill(0)).concat([new Array<number>(b.length + 1).fill(0)])
  for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1])
  const out: { add: boolean; l: string }[] = []
  let i = 0, j = 0
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) { i++; j++ }
    else if (j < b.length && (i >= a.length || lcs[i][j + 1] >= lcs[i + 1][j])) out.push({ add: true, l: b[j++] })
    else out.push({ add: false, l: a[i++] })
  }
  return out
}

/** Procedures learned from runs that worked. Proposed ones wait for you; a bot sees only the ones you accepted. */
/** Skills: the procedures your bots follow step by step, yours and the library's. */
export function SkillsView({ chatId }: { chatId: number }) {
  // Adding from the library shows the skill under Your skills straight away.
  const [added, setAdded] = useState(0)
  return (
    <>
      <div className="page-head"><div><h1 className="display">Skills</h1>
        <p className="lead">Step-by-step procedures your bots follow for a task. Add one from the library, teach one, or let a bot learn one from a task that went well. Use one on purpose by typing <code>/</code> in a chat.</p></div></div>
      <Skills key={added} chatId={chatId} />
      <SkillLibrary chatId={chatId} onAdded={() => setAdded(n => n + 1)} />
    </>
  )
}

/** A tool a skill needs, in words (for "Needs …"). */
const NEED: Record<string, string> = {
  calendar_list: 'a linked calendar', email_search: 'an email account', email_draft: 'an email account', meeting_read: 'a transcribed meeting', meeting_search: 'a transcribed meeting', web_fetch: 'reading web pages',
  routine_create: 'routines', chat_search: 'searching earlier chats', memory_search: 'memory', memory_store: 'memory',
}

/** Mimir's own skills: written for its tools, plain text (no scripts), and nothing is added until you add it. */
function SkillLibrary({ chatId, onAdded }: { chatId: number; onAdded: () => void }) {
  const lib = useLoad(() => invoke<LibrarySkill[]>('skill_library', { chatId }), [chatId])
  const bots = useLoad(() => invoke<Bot[]>('bots'), []).data ?? []
  const [target, setTarget] = useState(0)
  const [open, setOpen] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [scheduled, setScheduled] = useState<string[]>([])
  const [err, setErr] = useState<string | null>(null)
  const botOf = useContext(Bots)
  if (!lib.data) return null
  // A routine belongs to one bot: the one picked, or (for all bots) the first.
  const schedule = (k: LibrarySkill) => {
    if (!k.schedule) return
    setBusy(k.slug)
    invoke('add_routine', { chatId, botId: target || bots[0]?.id || 1, cron: k.schedule.cron, prompt: k.schedule.prompt, quiet: false })
      .then(() => { setErr(null); setScheduled(v => [...v, k.slug]) }, e => setErr(String(e))).finally(() => setBusy(null))
  }
  const cats = [...new Set(lib.data.map(k => k.category))]
  // For all bots: what any of them lacks.
  const missingFor = (k: LibrarySkill) => [...new Set(target === 0 ? Object.values(k.missing).flat() : k.missing[String(target)] ?? [])]
  const add = (k: LibrarySkill) => {
    setBusy(k.slug)
    invoke('skill_add', { chatId, slug: k.slug, botId: target }).then(() => { setErr(null); onAdded(); return lib.reload() }, e => setErr(String(e))).finally(() => setBusy(null))
  }
  return (
    <section className="library" aria-label="Skill library">
      <div className="section-title"><h2>From the library</h2><span className="count">{lib.data.length}</span>
        {bots.length > 1 && <select className="select select-inline" value={target} aria-label="Add skills for" onChange={e => setTarget(Number(e.target.value))}>
          <option value={0}>For all bots</option>{bots.map(b => <option key={b.id} value={b.id}>For {b.name}</option>)}
        </select>}</div>
      <p className="hint">Written for Mimir's tools. Adding one gives it to {target === 0 ? 'all your bots' : botOf(target).name} in this chat; you can change or remove it like any skill.</p>
      {err && <p className="error">{err}</p>}
      {cats.map(c => (
        <div key={c} className="lib-cat">
          <h3>{c}</h3>
          <div className="mem-list">{lib.data!.filter(k => k.category === c).map(k => {
            const has = k.added.includes(0) || k.added.includes(target)
            const missing = missingFor(k)
            return (
              <article className="mem" key={k.slug}>
                <div className="mem-actions lib-add">
                  {has ? <span className="badge live"><Icon name="check" size={11} />Added</span>
                    : <button className="btn btn-sm" disabled={busy === k.slug} onClick={() => add(k)}>{busy === k.slug ? 'Adding…' : 'Add'}</button>}
                </div>
                <div className="mem-title">{k.name}</div>
                <p className="mem-body">{k.description}</p>
                {open === k.slug && <div className="skill-steps selectable"><Markdown text={k.body} /></div>}
                <div className="mem-meta">
                  <button type="button" className="linkish" aria-expanded={open === k.slug} onClick={() => setOpen(open === k.slug ? null : k.slug)}>{open === k.slug ? 'hide steps' : 'see the steps'}</button>
                  {missing.length > 0 && <span className="warn" title="It can be added now; it works once these are there.">needs {[...new Set(missing.map(t => NEED[t] ?? t))].join(', ')}</span>}
                  {/* A habit: once added, run it on a schedule in one click (a routine whose prompt is the skill's command). */}
                  {has && k.schedule && (scheduled.includes(k.slug)
                    ? <span>scheduled {k.schedule.label.toLowerCase()} · change it in Routines</span>
                    : <button type="button" className="linkish" disabled={busy === k.slug} onClick={() => schedule(k)}>schedule it · {k.schedule.label.toLowerCase()}</button>)}
                </div>
              </article>
            )
          })}</div>
        </div>
      ))}
    </section>
  )
}

function Skills({ chatId }: { chatId: number }) {
  const list = useLoad(() => invoke<Skill[]>('skills', { chatId }), [chatId], 15000)
  const removed = useLoad(() => invoke<Skill[]>('removed_skills', { chatId }), [chatId, list.data?.map(k => `${k.id}:${k.status}`).join()])
  const [err, setErr] = useState<string | null>(null)
  const { leaving, leave } = useLeave()
  const botOf = useContext(Bots)
  const who = (id: number) => id === 0 ? 'All bots' : botOf(id).name
  const all = list.data ?? []
  const proposed = all.filter(k => k.status === 'proposed'), active = all.filter(k => k.status === 'active')
  const act = (id: number, p: () => Promise<unknown>) => leave(id, () => p().then(() => { setErr(null); return list.reload() }, e => setErr(String(e))))
  const replaces = (k: Skill) => active.some(a => a.bot_id === k.bot_id && a.name.toLowerCase() === k.name.toLowerCase())
  // Teach a skill from a link or pasted text (a proposal like the rest).
  const bots = useLoad(() => invoke<Bot[]>('bots'), []).data ?? []
  const [teaching, setTeaching] = useState<{ source: string; bot: number; busy: boolean } | null>(null)
  const [rehearsal, setRehearsal] = useState<{ id: number; r: Rehearsed | null } | null>(null)
  const rehearse = (k: Skill) => {
    setRehearsal({ id: k.id, r: null })
    invoke<Rehearsed>('rehearse', { chatId, botId: k.bot_id || bots[0]?.id || 1, skillId: k.id }).then(r => setRehearsal({ id: k.id, r }), e => { setRehearsal(null); setErr(String(e)) })
  }
  const teach = () => {
    if (!teaching?.source.trim()) return
    setTeaching({ ...teaching, busy: true })
    invoke('teach_skill', { chatId, botId: teaching.bot, source: teaching.source.trim() })
      .then(() => { setTeaching(null); setErr(null); list.reload() }, e => { setErr(String(e)); setTeaching(t => t && { ...t, busy: false }) })
  }
  return (
    <>
      <div className="teach">
        {teaching ? <form className="er" onSubmit={e => { e.preventDefault(); teach() }}>
          {bots.length > 1 && <select className="select" style={{ width: 'auto' }} value={teaching.bot} onChange={e => setTeaching({ ...teaching, bot: Number(e.target.value) })} aria-label="Which bot learns it">{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select>}
          <input className="input" autoFocus placeholder="A link, the text (how our expense process works, a checklist…), or a SKILL.md" aria-label="What to learn from" value={teaching.source} onChange={e => setTeaching({ ...teaching, source: e.target.value })} />
          <button className="btn btn-sm btn-ghost" type="button" onClick={() => setTeaching(null)}>Cancel</button>
          <button className="btn btn-sm" disabled={teaching.busy || !teaching.source.trim()}>{teaching.busy ? 'Learning…' : 'Learn'}</button>
        </form> : <button className="btn btn-sm btn-ghost" onClick={() => setTeaching({ source: '', bot: bots[0]?.id ?? 1, busy: false })}><Icon name="bolt" size={14} />Teach a skill</button>}
      </div>
      {proposed.length > 0 && (
        <section className="suggest" aria-label="Suggested skills">
          <div className="suggest-head"><Icon name="bolt" /><b>{proposed.length} suggested {proposed.length === 1 ? 'skill' : 'skills'}</b>
            <small>From tasks that went well, fixes after something went wrong, and upkeep. Nothing changes until you accept.</small></div>
          {proposed.map(k => {
            const current = active.find(a => a.bot_id === k.bot_id && a.name.toLowerCase() === k.name.toLowerCase())
            const merging = active.filter(a => k.replaces.includes(a.id))
            return (
              <div className={'sugg' + (leaving.has(k.id) ? ' leaving' : '')} key={k.id}>
                <span className="badge live">{k.kind === 'saved' ? 'Saved from chat' : k.kind === 'taught' ? (current ? 'Taught again' : 'Taught') : k.kind === 'patch' ? 'Fix' : k.kind === 'merge' ? 'Merge' : k.kind === 'retire' ? 'Retire' : replaces(k) ? 'New version' : 'Skill'}</span>
                <div className="what selectable"><b>{k.name}</b>{k.description}
                  {(k.kind === 'patch' || k.kind === 'taught') && current && lineChanges(current.body, k.body).length > 0
                    ? <div className="skill-steps skill-diff">{lineChanges(current.body, k.body).map((c, i) => <div key={i} className={c.add ? 'add' : 'del'}>{c.add ? '+ ' : '− '}{c.l}</div>)}</div>
                    : k.kind === 'retire' ? null : <div className="skill-steps"><Markdown text={k.body} /></div>}
                  {k.kind === 'merge' && merging.length > 0 && <div className="why">Replaces {merging.map(m => `“${m.name}”`).join(' and ')}</div>}
                  {k.reason && <div className="why">{k.reason}</div>}
                  <div className="why">For {who(k.bot_id)}{k.run_id ? `, learned from run #${k.run_id}` : ''}</div></div>
                <div className="choices">
                  <button className="btn btn-sm btn-ghost" onClick={() => act(k.id, () => invoke('decide_skill', { chatId, id: k.id, accept: false }))}>{k.kind === 'retire' ? 'Keep it' : 'Dismiss'}</button>
                  <button className="btn btn-sm" onClick={() => act(k.id, () => invoke('decide_skill', { chatId, id: k.id, accept: true }))}>{k.kind === 'retire' ? 'Retire' : k.kind === 'patch' ? 'Apply fix' : 'Accept'}</button>
                </div>
              </div>
            )
          })}
        </section>
      )}
      {active.length > 0 && (
        <section>
          <div className="section-title"><h2>Your skills</h2><span className="count">{active.length}</span></div>
          <div className="mem-list">{active.map(k => (
            <article className={'mem' + (leaving.has(k.id) ? ' leaving' : '')} key={k.id}>
              <div className="mem-actions">
                <button className="btn btn-ghost btn-sm" disabled={rehearsal?.id === k.id && !rehearsal.r} title="Try it once without sending or changing anything: see what it would do" onClick={() => rehearse(k)}>Rehearse</button>
                <button className="btn btn-ghost btn-sm" title="Copy as a SKILL.md file, for another agent or to share"
                  onClick={() => copyText(invoke<string>('export_skill', { chatId, id: k.id })).then(() => setErr(null), e => setErr(String(e)))}>Copy as SKILL.md</button>
                <button className="btn btn-ghost btn-danger btn-sm icon-btn" title="Remove" aria-label={`Remove ${k.name}`}
                  onClick={() => act(k.id, () => invoke('remove_skill', { chatId, id: k.id }))}><Icon name="trash" size={14} /></button>
              </div>
              <div className="mem-title">{k.name}</div>
              <p className="mem-body">{k.description}</p>
              <div className="skill-steps selectable"><Markdown text={k.body} /></div>
              {rehearsal?.id === k.id && (rehearsal.r ? <RehearsalResult r={rehearsal.r} onClose={() => setRehearsal(null)} /> : <p className="hint">Rehearsing… it runs for real, but anything it would send or change is only described.</p>)}
              <div className="mem-meta">
                {bots.length > 1
                  ? <select className="select select-inline" value={k.bot_id} aria-label={`Which bots use ${k.name}`}
                      onChange={e => invoke('share_skill', { chatId, id: k.id, botId: Number(e.target.value) }).then(() => { setErr(null); list.reload() }, er => setErr(String(er)))}>
                      <option value={0}>All bots</option>{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}
                    </select>
                  : <span>{who(k.bot_id)}</span>}
                <span>{ago(k.created_at)}</span>
                <span>{k.uses ? `used ${k.uses}× · last ${ago(k.last_used ?? k.created_at)}` : 'not used yet'}</span></div>
            </article>
          ))}</div>
        </section>
      )}
      {(removed.data?.length ?? 0) > 0 && (
        <details className="retired">
          <summary className="hint">{removed.data!.length} retired or removed {removed.data!.length === 1 ? 'skill' : 'skills'}</summary>
          <div className="mem-list">{removed.data!.map(k => (
            <article className="mem" key={k.id}>
              <div className="mem-actions"><button className="btn btn-ghost btn-sm" onClick={() => invoke('restore_skill', { chatId, id: k.id }).then(() => { setErr(null); list.reload() }, e => setErr(String(e)))}>Bring back</button></div>
              <div className="mem-title">{k.name}</div>
              <p className="mem-body">{k.description}</p>
              <div className="mem-meta"><span>{who(k.bot_id)}</span><span>{k.uses ? `used ${k.uses}×` : 'never used'}</span></div>
            </article>
          ))}</div>
        </details>
      )}
      {err && <p className="error">{err}</p>}
    </>
  )
}
