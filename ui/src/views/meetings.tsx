import { useEffect, useRef, useState } from 'react'
import { copyText, invoke, keystore, localInvoke, platform } from '../api'
import { Markdown } from '../markdown'
import { ask, askText, tell, Icon, Switch } from '../ui'
import { useLoad, LOCAL } from '../shared'
import type { Bot } from '../types'

type MeetingLine = { ts: number; who: string; text: string }

type MeetingStatus = { recording: boolean; started_at: number; lines: MeetingLine[]; sources: string[]; pending: number; error?: string | null; warning?: string | null; mic_warning?: string | null; preparing?: string | null; meeting_id?: number | null }

type MeetingRow = { id: number; title: string; started_at: number; ended_at: number; words: number; notes: string; speakers?: Record<string, string>; summary?: string }

/** "You", a name the person gave, "Speaker 2" once voices are told apart, else "Others". */
const speaker = (who: string, names: Record<string, string> = {}) =>
  who === 'you' ? 'You' : who.startsWith('others:') ? names[who] ?? `Speaker ${who.slice(7)}` : 'Others'

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

/** A transcript as plain text, for copying and for handing to a bot. */
const transcriptText = (lines: MeetingLine[], start: number, names: Record<string, string> = {}) =>
  lines.map(l => `[${mmss(Math.max(0, l.ts - start))}] ${l.who === 'you' ? 'Me' : speaker(l.who, names)}: ${l.text}`).join('\n')

/** Meetings and calls, transcribed on this computer: your microphone is "you", the computer's sound is the others. */
export function MeetingsView({ bots, onSent }: { bots: Bot[]; onSent: (bot: number) => void }) {
  const [st, setSt] = useState<MeetingStatus | null>(null)
  const [engine, setEngine] = useState<{ engine: string | null; model: string; model_ready: boolean } | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [open, setOpen] = useState<number | null>(null)
  const [offer, setOffer] = useState<boolean | null>(null)
  // The calendar's private iCal link: '' (none), 'linked', or the text being pasted.
  const [cal, setCal] = useState('')
  const [calNote, setCalNote] = useState<string | null>(null)
  const linkCalendar = (url: string) => localInvoke<{ linked: boolean; this_week?: number }>('meeting', { action: 'calendar', url })
    .then(r => { setCal(r.linked ? 'linked' : ''); setCalNote(r.linked ? `${r.this_week} ${r.this_week === 1 ? 'meeting' : 'meetings'} this week.` : null) }, e => setCalNote(String(e)))
  const [mics, setMics] = useState<{ mics: string[]; chosen: string | null } | null>(null)
  const [notes, setNotes] = useState('')
  const [localBots, setLocalBots] = useState<Bot[]>([])
  const setting = (key: string, value: string) => localInvoke('meeting', { action: 'set', [key]: value }).catch(e => setErr(String(e)))
  // Saved on this computer as they're transcribed (even when the app shows a server).
  const list = useLoad(() => localInvoke<MeetingRow[]>('meeting', { action: 'list' }), [], 15000)
  const [, tick] = useState(0)
  useEffect(() => {
    localInvoke<typeof engine>('meeting', { action: 'engine' }).then(setEngine, () => {})
    localInvoke<boolean>('meeting', { action: 'offer' }).then(setOffer, () => {})
    localInvoke<{ linked: boolean }>('meeting', { action: 'calendar' }).then(c => setCal(c.linked ? 'linked' : ''), () => {})
    localInvoke<{ mics: string[]; chosen: string | null }>('meeting', { action: 'mics' }).then(setMics, () => {})
    localInvoke<{ meeting_notes: string | null }>('meeting', { action: 'set' }).then(s => setNotes(s.meeting_notes ?? ''), () => {})
    // The bots on this computer (they write notes when a meeting ends, here, even when the app shows a server).
    localInvoke<Bot[]>('bots').then(setLocalBots, () => {})
    let live = true
    let first = true
    const poll = () => localInvoke<MeetingStatus>('meeting', { action: 'status' }).then(s => {
      if (!live) return
      // Opened from the tray's "Stop transcribing": show the meeting that just ended.
      if (first && !s.recording && s.meeting_id) setOpen(s.meeting_id)
      first = false; setSt(s); tick(n => n + 1)
    }, () => {}).finally(() => { if (live) setTimeout(poll, 1200) })
    poll()
    return () => { live = false }
  }, [])
  const start = () => {
    setBusy(true); setErr(null)
    localInvoke<MeetingStatus>('meeting', { action: 'start' }).then(setSt, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const stop = () => {
    setBusy(true)
    localInvoke<MeetingStatus>('meeting', { action: 'stop' }).then(s => { setSt(s); list.reload(); setOpen(s.meeting_id ?? null) }, e => setErr(String(e))).finally(() => setBusy(false))
  }
  const recording = !!st?.recording
  const saved = (list.data ?? []).filter(m => !recording || m.id !== st!.meeting_id)
  // Search every transcript ("deck", a name, a topic): matching lines, each opening its meeting.
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<(MeetingLine & { id: number; title: string; started_at: number })[]>([])
  useEffect(() => {
    if (!query.trim()) { setHits([]); return }
    const t = setTimeout(() => localInvoke<typeof hits>('meeting', { action: 'search', query: query.trim() }).then(setHits, () => {}), 200)
    return () => clearTimeout(t)
  }, [query])
  return (
    <>
      <div className="page-head"><div><h1 className="display">Meetings</h1>
        <p className="lead">Transcribe meetings and phone calls on this computer: Teams, Zoom, Meet, Webex, Slack, WhatsApp, Teams or Zoom Phone, RingCentral and other softphones. Then hand it to a bot for a summary and follow-ups.</p></div></div>
      {err && <p className="error">{err}</p>}
      {engine && !engine.engine && (
        <div className="howto"><Icon name="alert" /><span>To transcribe, install whisper.cpp ({platform === 'mac' ? <code>brew install whisper-cpp</code> : <>put <code>whisper-cli</code> from github.com/ggml-org/whisper.cpp on your PATH</>}; it runs on this computer, private and free), or add an OpenAI-compatible key in Settings.</span></div>
      )}
      <div className={'meeting-card' + (recording ? ' live' : '')}>
        {recording ? <>
          <div className="mc-head"><span className="rec-dot" /><b>Transcribing</b><span className="mono">{mmss(Date.now() / 1000 - st!.started_at)}</span>
            <span className="hint">{st!.preparing ?? st!.sources.join(' + ') + (st!.pending ? ' · writing…' : '')}</span>
            <button className="btn btn-primary" disabled={busy} onClick={stop}>{busy ? 'Finishing…' : 'Stop'}</button></div>
          <p className="hint" style={{ margin: 0 }}>New lines appear a few seconds after each pause. Let the others know you're transcribing.</p>
        </> : <>
          <div className="mc-head"><b>Start when the meeting starts</b>
            <button className="btn btn-primary" disabled={busy || (engine !== null && !engine.engine)} onClick={start}><Icon name="mic" size={14} />{busy ? 'Starting…' : 'Start transcribing'}</button></div>
          <p className="hint" style={{ margin: 0 }}>Records your microphone (you) and this computer's sound (everyone else), and transcribes {engine?.engine === 'local' ? `here with whisper.cpp (${engine.model} model${engine.model_ready ? '' : ', downloads once'})` : 'with your OpenAI-compatible key'}. Saved as it goes. Headphones keep the two sides apart. A call on your mobile? Take it on the computer ({platform === 'win' ? 'Phone Link' : platform === 'mac' ? 'iPhone calls on your Mac' : 'or put it on speaker, then it all reads as You'}) and it's transcribed the same way. Recording a call usually needs the others' consent.</p>
          <div className="mc-opts">
            {(mics?.mics.length ?? 0) > 1 && <label>Your microphone
              <select className="select" value={mics!.chosen ?? ''} onChange={e => { setMics({ ...mics!, chosen: e.target.value }); setting('meeting_mic', e.target.value) }}>
                <option value="">System default</option>{mics!.mics.map(m => <option key={m}>{m}</option>)}
              </select></label>}
            {localBots.length > 0 && <label>When it ends
              <select className="select" value={notes} onChange={e => { setNotes(e.target.value); setting('meeting_notes', e.target.value) }}>
                <option value="">Just save the transcript</option>{localBots.map(b => <option key={b.id} value={String(b.id)}>{b.name} writes the notes</option>)}
              </select></label>}
          </div>
          <div className="mc-cal">
            <span>Calendar <span className="hint">· names recordings after the meeting, knows who's invited, and offers to transcribe when one starts</span></span>
            {cal === 'linked'
              ? <div className="er"><span className="hint grow">Linked (the link is kept in {keystore})</span><button className="btn btn-ghost btn-sm" onClick={() => linkCalendar('')}>Unlink</button></div>
              : <form className="er" onSubmit={e => { e.preventDefault(); if (cal.trim()) linkCalendar(cal.trim()) }}>
                  <input className="input" type="url" placeholder="Your calendar's secret iCal link (Google: Settings → your calendar → Secret address in iCal format)" aria-label="Calendar iCal link" value={cal} onChange={e => setCal(e.target.value)} />
                  <button className="btn btn-sm" disabled={!cal.trim()}>Link</button>
                </form>}
            {calNote && <span className="hint">{calNote}</span>}
          </div>
          <label className="mc-offer"><Switch label="Offer to transcribe when a call starts" checked={!!offer} disabled={offer === null}
            onChange={v => localInvoke<boolean>('meeting', { action: 'offer', on: v }).then(setOffer, e => setErr(String(e)))} />
            <span>Offer to transcribe when a call starts <span className="hint">(a notification once another app has used the microphone for 20 seconds)</span></span></label>
        </>}
        {!recording && st?.preparing && <p className="hint" style={{ margin: 0 }}>{st.preparing}</p>}
        {st?.error && <p className="error" style={{ margin: 0 }}>{st.error}</p>}
        {recording && [st!.mic_warning, st!.warning].filter(Boolean).map(w => <div key={w!} className="howto"><Icon name="alert" /><span>{w}</span></div>)}
      </div>
      {recording && st!.meeting_id && <LiveNotes id={st!.meeting_id} />}
      {recording && st!.lines.length > 0 && <Transcript lines={st!.lines} start={st!.started_at} />}
      {saved.length > 0 && <>
        <div className="section-title"><h2>Saved</h2><span className="count">{saved.length}</span>
          <input className="input meeting-search" type="search" placeholder="Search what was said" aria-label="Search meetings" value={query} onChange={e => setQuery(e.target.value)} /></div>
        {query.trim() && <div className="transcript">{hits.length ? hits.map((h, i) => (
          <button key={i} type="button" className="tl hit" onClick={() => { setOpen(h.id); setQuery('') }}>
            <span className="mono">{mmss(Math.max(0, h.ts - h.started_at))}</span><b>{speaker(h.who, list.data?.find(m => m.id === h.id)?.speakers)}</b>
            <span>{h.text} <span className="hint">· {meetingTitle(h)}</span></span>
          </button>
        )) : <span className="hint">Nothing said matches.</span>}</div>}
        <div className="mem-list">{saved.map(m => (
          <SavedMeeting key={m.id} m={m} open={open === m.id} toggle={() => setOpen(open === m.id ? null : m.id)} bots={bots} onSent={onSent} onChange={list.reload} />
        ))}</div>
      </>}
    </>
  )
}

/** A notepad during the call: a few words on what matters, saved as you type; the notes bot builds around them. */
function LiveNotes({ id }: { id: number }) {
  const [text, setText] = useState('')
  const [saved, setSaved] = useState(true)
  // What's typed but not yet saved, for the save on the way out: Stop hides this box at once, often inside the 700 ms.
  const unsaved = useRef<string | null>(null)
  useEffect(() => {
    if (saved) return
    const t = setTimeout(() => localInvoke('meeting', { action: 'notes', id, notes: text }).then(() => { unsaved.current = null; setSaved(true) }, () => {}), 700)
    return () => clearTimeout(t)
  }, [text, saved, id])
  useEffect(() => () => { if (unsaved.current !== null) localInvoke('meeting', { action: 'notes', id, notes: unsaved.current }).catch(() => {}) }, [id])
  return (
    <label className="live-notes">
      <span>Your notes <span className="hint">{text ? (saved ? '· saved' : '· saving…') : '· a few words on what matters; the notes build around them'}</span></span>
      <textarea className="input meeting-notes" rows={4} value={text} placeholder={'- budget needs sign-off\n- I own the landing page'} onChange={e => { setText(e.target.value); unsaved.current = e.target.value; setSaved(false) }} />
    </label>
  )
}

function Transcript({ lines, start, names, onName }: { lines: MeetingLine[]; start: number; names?: Record<string, string>; onName?: (key: string) => void }) {
  return (
    <div className="transcript selectable">{lines.map((l, i) => (
      <div key={i} className={'tl ' + (l.who === 'you' ? 'you' : 'others')}><span className="mono">{mmss(Math.max(0, l.ts - start))}</span>
        {/* the name once per turn, not on every sentence; a told-apart speaker can be named */}
        {lines[i - 1]?.who === l.who ? <b /> : onName && l.who.startsWith('others:')
          ? <button type="button" className="tl-name" title="Name this speaker" onClick={() => onName(l.who)}>{speaker(l.who, names)}</button>
          : <b>{speaker(l.who, names)}</b>}
        <span>{l.text}</span></div>
    ))}</div>
  )
}

const meetingTitle = (m: Pick<MeetingRow, 'title' | 'started_at'>) => m.title || `Meeting, ${new Date(m.started_at * 1000).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}`

function SavedMeeting({ m, open, toggle, bots, onSent, onChange }: { m: MeetingRow; open: boolean; toggle: () => void; bots: Bot[]; onSent: (bot: number) => void; onChange: () => void }) {
  const detail = useLoad(() => open ? localInvoke<{ lines: MeetingLine[] }>('meeting', { action: 'get', id: m.id }) : Promise.resolve(null), [open, m.id], 60000)
  const [title, setTitle] = useState(m.title)
  const [jotted, setJotted] = useState(m.notes ?? '')
  const rename = () => { if (title.trim() !== m.title) localInvoke('meeting', { action: 'rename', id: m.id, title: title.trim() }).then(onChange, e => setNote(String(e))) }
  const [bot, setBot] = useState(bots[0]?.id ?? 1)
  const [note, setNote] = useState<string | null>(null)
  const lines = detail.data?.lines ?? []
  const send = () => {
    // What you wrote is yours; the transcript (and the invite's title) are other people's words, so they go as an
    // attached file: outside content, which the bot reads but doesn't take orders from.
    const said = `Summarise my meeting from ${new Date(m.started_at * 1000).toLocaleString()} (${mmss(m.ended_at - m.started_at)} long), attached: `
      + 'decisions, action items with who owns them, and anything I promised.'
      + (jotted.trim() ? `\n\nMy notes during the call (what mattered to me; build around them):\n${jotted.trim()}` : '')
    const transcript = `Meeting: ${title.trim() || meetingTitle(m)}\n\n${transcriptText(lines, m.started_at, m.speakers)}`
    // Base64 in pieces: one call over an hour's transcript would overflow the stack.
    const bytes = new TextEncoder().encode(transcript)
    let bin = ''
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
    const data = btoa(bin)
    invoke<string>('read_document', { botId: bot, chatId: LOCAL, name: 'meeting transcript.txt', data, text: said })
      .then(text => invoke<string>('send_local', { botId: bot, text, images: [] }))
      // The bot's reply is kept on the meeting too (the chat shows it as it's written).
      .then(reply => localInvoke('meeting', { action: 'summary', id: m.id, summary: reply }).then(onChange))
      .catch(e => setNote(String(e)))
    // To the chat with the bot that's writing the notes, so its reply is what you see.
    onSent(bot)
  }
  return (
    <article className="mem">
      <button type="button" className="meeting-row" onClick={toggle} aria-expanded={open}>
        <span className="mem-title">{meetingTitle(m)}</span>
        <span className="hint">{new Date(m.started_at * 1000).toLocaleString()} · {mmss(m.ended_at - m.started_at)} · {m.words} words</span>
      </button>
      {open && <>
        <input className="input" style={{ marginTop: 10 }} placeholder="Name it, e.g. Weekly standup" aria-label="Meeting title" value={title}
          onChange={e => setTitle(e.target.value)} onBlur={rename} onKeyDown={e => e.key === 'Enter' && e.currentTarget.blur()} autoFocus={!m.title} />
        {m.summary && <div className="meeting-summary selectable"><Markdown text={m.summary} /></div>}
        <textarea className="input meeting-notes" placeholder="Your notes (what mattered to you)" aria-label="Your notes" value={jotted} rows={Math.min(8, Math.max(2, jotted.split('\n').length))}
          onChange={e => setJotted(e.target.value)} onBlur={() => { if (jotted !== m.notes) localInvoke('meeting', { action: 'notes', id: m.id, notes: jotted }).then(onChange, e => setNote(String(e))) }} />
        {lines.length ? <Transcript lines={lines} start={m.started_at} names={m.speakers} onName={key => {
          askText(`Who is ${speaker(key, m.speakers)}?`, m.speakers?.[key] ?? '').then(name => {
            if (name !== null) localInvoke('meeting', { action: 'speaker', id: m.id, key, name }).then(onChange, e => setNote(String(e)))
          })
        }} /> : <p className="hint">Nothing was said, or it was too quiet to transcribe.</p>}
        <div className="er" style={{ marginTop: 10 }}>
          {bots.length > 1 && <select className="select" style={{ width: 'auto' }} value={bot} onChange={e => setBot(Number(e.target.value))} aria-label="Which bot">{bots.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select>}
          <button className="btn btn-primary" disabled={!lines.length} onClick={send}>{m.summary ? 'Summarise again with' : 'Summarise with'} {bots.find(b => b.id === bot)?.name ?? 'your bot'}</button>
          <button className="btn btn-ghost" disabled={!lines.length} onClick={() => copyText(transcriptText(lines, m.started_at, m.speakers)).then(() => setNote('Copied'))}>Copy</button>
          <span className="grow" />
          <button className="btn btn-ghost btn-danger" onClick={() => ask(`Delete “${meetingTitle(m)}”?`, 'Delete', true).then(ok => { if (ok) localInvoke('meeting', { action: 'delete', id: m.id }).then(onChange, e => tell(`Couldn't delete it: ${e}`)) })}>Delete</button>
        </div>
        {note && <p className="hint">{note}</p>}
      </>}
    </article>
  )
}
