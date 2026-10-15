import { Component, type ReactNode, useMemo, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { inDesktop, invoke, localInvoke, probe, remoteServer, saveDownload, setWebToken, shortcut, webToken } from './api'
import { moodOf, type Mood } from './face'
import { ahead, setLooks } from './format'
import { ask, AskHost, tell, Avatar, Icon, Segmented } from './ui'
import { CHANNEL, useLoad, LOCAL, HOUSEHOLD, Bots, online, presenceOf } from './shared'
import type { Tab, Status, Busy, Bot, Chat, Inbox, ComputerStatus, Pause } from './types'
import { ActivityView } from './views/activity'
import { BotImport, BotPage } from './views/bot'
import { Paused, pausedText, ChatView, Menu, pauseChoices } from './views/chat'
import { GoalLine } from './views/cards'
import { ComputerView } from './views/computer'
import { inboxCount, Approvals, InboxView, SignInForm } from './views/inbox'
import { MeetingsView } from './views/meetings'
import { MemoryView } from './views/memory'
import { Onboarding } from './views/onboarding'
import { RoutinesView } from './views/routines'
import { SettingsView } from './views/settings'
import { SkillsView } from './views/skills'

type View = Tab | 'settings' | 'bot' | 'inbox' | 'computer' | 'meetings'

/** The built-in chat: on this computer, or the web chat of a self-hosted server. */
const LOCAL_NAME = inDesktop && !remoteServer() ? 'This computer' : 'Web chat'

/** What's wrong with a bot that isn't working, in words (its account, or a messaging app that won't connect). */
const troubleOf = (s?: Status): string | null =>
  s?.error ?? (s?.channels.filter(c => c.error && !c.connected)
    .map(c => (c.error!.startsWith(CHANNEL[c.kind].name) ? c.error! : `${CHANNEL[c.kind].name}: ${c.error}`)).join(' · ') || null)

/** A bot at work, as its face: the step a reply is on, else what kind of work it is. */
const busyMood = (w: Busy): Mood => w.step ? moodOf(w.step, '') : w.trigger === 'tidy' || w.trigger === 'import' || w.trigger === 'know_me' ? 'remembering' : w.trigger === 'teach' ? 'writing' : 'thinking'

const BUSY_KIND: Record<string, string> = { routine: 'Running a routine', delegated: 'On a task for another bot', reported: 'Reporting back on a task', tidy: 'Tidying memory', check: 'Checking a preference', import: 'Importing memories', know_me: 'Getting to know you', teach: 'Writing a skill' }

const busyLabel = (w: Busy) => w.step ? `${w.step}…` : BUSY_KIND[w.trigger] ?? 'Replying…'

/** Browser on a self-hosted server: ask for the admin token first. */
export default function Root() {
  const [authed, setAuthed] = useState(inDesktop || !!webToken())
  useEffect(() => {
    const out = () => { if (!inDesktop) { setWebToken(null); setAuthed(false) } }
    window.addEventListener('mimir-unauthorized', out)
    return () => window.removeEventListener('mimir-unauthorized', out)
  }, [])
  return authed ? <App /> : <SignIn onDone={() => setAuthed(true)} />
}

function SignIn({ onDone }: { onDone: () => void }) {
  const [token, setToken] = useState('')
  const [err, setErr] = useState<string | null>(null)
  return (
    <div className="center-screen">
      <form className="signin" onSubmit={e => {
        e.preventDefault()
        probe('', token.trim()).then(() => { setWebToken(token.trim()); onDone() }, e => setErr(String(e)))
      }}>
        <img className="mark" src="/logo.png" alt="" />
        <h1>Sign in to your bots</h1>
        <p>Run <code>mimir admin-token</code> on your server to see the token.</p>
        <input className="input" type="password" aria-label="Admin token" placeholder="Admin token" value={token} onChange={e => setToken(e.target.value)} autoFocus autoComplete="current-password" />
        <button className="btn btn-primary btn-lg" style={{ justifyContent: 'center' }} disabled={!token.trim()}>Sign in</button>
        {err && <p className="error">{err}</p>}
      </form>
    </div>
  )
}

// One UI bug costs a click, not a dead window.
class Crashed extends Component<{ children: ReactNode }, { err: string | null }> {
  state = { err: null as string | null }
  static getDerivedStateFromError(e: unknown) { return { err: String(e) } }
  render() {
    if (!this.state.err) return this.props.children
    return <div className="empty"><p>Something went wrong on this screen.</p><p className="mono">{this.state.err}</p><button type="button" className="btn btn-primary" onClick={() => location.reload()}>Reload</button></div>
  }
}

function App() {
  const [onboarded, setOnboarded] = useState<boolean | null>(null)
  useEffect(() => { invoke<{ onboarded: boolean }>('info').then(i => setOnboarded(i.onboarded), () => setOnboarded(true)) }, [])
  if (onboarded === null) return null
  return <><AskHost />{onboarded ? <Crashed><Main /></Crashed> : <Onboarding onDone={() => setOnboarded(true)} />}</>
}

const TABS: { id: Tab; label: string; icon: string; hint: string }[] = [
  { id: 'chat', label: 'Chat', icon: 'chat', hint: `Chat  ${shortcut('1')}` }, { id: 'memory', label: 'Memory', icon: 'memory', hint: `Memory  ${shortcut('2')}` },
  { id: 'skills', label: 'Skills', icon: 'bolt', hint: `Skills  ${shortcut('3')}` },
  { id: 'routines', label: 'Routines', icon: 'clock', hint: `Routines  ${shortcut('4')}` }, { id: 'activity', label: 'Activity', icon: 'activity', hint: `Activity  ${shortcut('5')}` },
]

function Main() {
  const bots = useLoad(() => invoke<Bot[]>('bots'), [])
  const statuses = useLoad(() => invoke<Status[]>('statuses'), [], 2000)
  // A bot another bot just made shows up in the statuses first: then the list reloads.
  useEffect(() => {
    if (bots.data && statuses.data?.some(s => !bots.data!.some(b => b.id === s.bot_id))) bots.reload()
  }, [statuses.data, bots.data]) // eslint-disable-line react-hooks/exhaustive-deps
  // What each bot is working on right now (a reply, a routine, a task from another bot, a memory tidy-up).
  const busy = useLoad(() => invoke<Busy[]>('busy'), [], 1500)
  const chats = useLoad(() => invoke<Chat[]>('chats'), [], 15000) // someone new writing to a bot adds a chat
  // Approvals waiting come with the inbox.
  const inbox = useLoad(() => invoke<Inbox>('inbox'), [], 2000)
  const approvals = inbox.data?.approvals ?? []
  // Bots that aren't working (an account or a messaging app that won't connect) need you too.
  const troubled = (statuses.data ?? []).flatMap(s => { const t = troubleOf(s); return t ? [{ id: s.bot_id, text: t }] : [] })
  const needs = inboxCount(inbox.data) + troubled.length
  const [chatId, setChatId] = useState<number>(LOCAL)
  const [botId, setBotId] = useState<number>(1) // for the bot page; 0 = new bot
  const [incognito, setIncognito] = useState(false)
  // Incognito ends when you go to another chat.
  useEffect(() => setIncognito(false), [chatId])
  // A link to the live computer (a bot asking you to take over) opens it: #computer. "Edit in Mimir" on a chat app's
  // card opens Needs you, where the card is: #approval-<id>. A sign-in card's opens the sign-in form: #signin.
  const linked = (): View | null => location.hash === '#computer' ? 'computer' : location.hash.startsWith('#approval-') ? 'inbox' : null
  const [view, setView] = useState<View>(() => linked() ?? 'chat')
  const [signIn, setSignIn] = useState(false)
  useEffect(() => {
    const go = () => {
      const v = linked()
      if (v) setView(v)
      else if (location.hash === '#signin') setSignIn(true)
      else return
      history.replaceState(null, '', location.pathname + location.search)
    }
    go()
    window.addEventListener('hashchange', go)
    return () => window.removeEventListener('hashchange', go)
  }, [])
  // Tell the server where its web app is reached, for links in messages (only a server keeps it).
  useEffect(() => {
    const at = inDesktop ? remoteServer()?.url : location.origin
    if (at && (!inDesktop || remoteServer())) invoke('public_url', { url: at }).catch(() => {})
  }, [])
  // Content drifts in from the side of the tab you moved to (left or right); other views just rise.
  const lastTab = useRef(0)
  const tabIndex = TABS.findIndex(t => t.id === view)
  const dx = useMemo(() => (tabIndex < 0 ? 0 : Math.sign(tabIndex - lastTab.current) * 14), [view]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (tabIndex >= 0) lastTab.current = tabIndex }, [tabIndex])
  // The sidebar's selection is one highlight that glides to the chosen bot or chat.
  const railRef = useRef<HTMLDivElement>(null)
  const railOn = useRef<HTMLElement | null>(null)
  useLayoutEffect(() => {
    const el = railRef.current
    if (!el) return
    const on = el.querySelector<HTMLElement>('.rail-item.on')
    el.classList.toggle('has-glide', !!on)
    if (!on) return
    // Glide only when the selection changes; when the list itself shifts (bots loading), jump.
    const jump = railOn.current === null || railOn.current === on
    railOn.current = on
    if (jump) el.classList.add('measuring')
    el.style.setProperty('--gy', `${on.offsetTop}px`)
    el.style.setProperty('--gh', `${on.offsetHeight}px`)
    if (jump) requestAnimationFrame(() => el.classList.remove('measuring'))
  })

  const botList = bots.data ?? []
  setLooks(botList) // every avatar of a bot shows the creature you chose for it
  const botOf = (id: number) => botList.find(b => b.id === id) ?? { id, name: `Bot ${id}` }
  const statusOf = (id: number) => statuses.data?.find(s => s.bot_id === id)
  const all = chats.data ?? []
  // The built-in chat first, then the household, then the paired chats.
  const list = (all.some(c => c.chat_id === LOCAL) ? all : [{ chat_id: LOCAL, title: LOCAL_NAME, memories: 0, channel: 'local' }, ...all])
    .toSorted((a, b) => Number(b.chat_id === LOCAL) - Number(a.chat_id === LOCAL) || Number(b.chat_id === HOUSEHOLD) - Number(a.chat_id === HOUSEHOLD))
  const chat = list.find(c => c.chat_id === chatId) ?? list[0]
  // The household is memories only: nobody chats there.
  const household = chat.channel === 'household'
  useEffect(() => { if (household && view !== 'memory' && TABS.some(t => t.id === view)) setView('memory') }, [household, view])
  const inChat = view !== 'settings' && view !== 'bot' && view !== 'inbox' && view !== 'computer' && view !== 'meetings'
  const computer = useLoad(() => invoke<ComputerStatus>('computer'), [], 20000)
  // A meeting being transcribed (this computer), for the sidebar's dot.
  const meetingNow = useLoad(() => inDesktop ? localInvoke<{ recording: boolean }>('meeting', { action: 'status' }) : Promise.resolve({ recording: false }), [], 3000)
  const recording = !!meetingNow.data?.recording
  const openBot = (id: number) => { setBotId(id); setView('bot'); setMenu(null) }
  // From elsewhere (Needs you): the chat as a whole, not the bot view you had open before.
  const openChat = (id: number, v: Tab) => { setChatId(id); setViaBot(false); setView(v) }
  // Clicking a bot opens a chat with it (on this computer); its settings are a right-click (or the gear by the
  // composer's bot picker) away.
  const [chatBot, setChatBot] = useState<number | null>(null)
  const [viaBot, setViaBot] = useState(false)
  const talkTo = (id: number) => { setChatBot(id); setChatId(LOCAL); setIncognito(false); setViaBot(true); setView('chat'); setMenu(null) }
  const botChat = viaBot && view === 'chat' && chat.chat_id === LOCAL
  // A bot's right-click menu, or pause choices (for one bot, or all of them: id null).
  const [menu, setMenu] = useState<{ x: number; y: number; id: number | null; pick?: boolean } | null>(null)
  // A bot file being read before it's added (from a file, or a bot's own for Duplicate).
  const [importing, setImporting] = useState<{ text: string; duplicateOf?: number } | null>(null)
  const pauses = useLoad(() => invoke<Pause[]>('pause'), [], 5000)
  const allPause = pauses.data?.find(p => p.bot_id === null)
  const ownPause = (id: number) => pauses.data?.find(p => p.bot_id === id)
  const pausedOf = (id: number) => {
    const u = [allPause, ownPause(id)].flatMap(p => p ? [p.until] : [])
    return u.length ? (u.includes(0) ? 0 : Math.max(...u)) : undefined
  }
  const setPause = (botId: number | null, on: boolean, until = 0) => { setMenu(null); invoke('pause', { botId, on, until }).then(pauses.reload, e => tell(String(e))) }
  // On a phone the sidebar is a full screen of its own, opened from the tab bar's Chats.
  const [phoneList, setPhoneList] = useState(false)
  const phoneTab = phoneList || view === 'chat' ? 'chats' : view
  const go = (v: View) => { setPhoneList(false); setView(v) }
  // A bot pointing at a setting ("Settings → Email") opens Settings searched for it.
  const [find, setFind] = useState('')
  useEffect(() => { if (view !== 'settings') setFind('') }, [view])
  const openSettings = (q: string) => { setFind(q); go('settings') }

  // ⌘F: on the Settings page it searches settings; anywhere else, memory.
  const findIt = () => {
    // In Settings or a chat, its own search box; elsewhere, Memory's.
    const box = document.querySelector<HTMLInputElement>('.settings-search input, .chat-search input')
    if (box) { box.focus(); box.select(); return }
    setView('memory')
    setTimeout(() => document.querySelector<HTMLInputElement>('.search input')?.focus(), 50)
  }
  // The macOS menu bar sends its commands here (same actions as the shortcuts below).
  useEffect(() => {
    if (!inDesktop) return
    let off: (() => void) | undefined
    import('@tauri-apps/api/event').then(({ listen }) => listen<string>('menu', e => {
      const id = e.payload
      if (id === 'new_bot') { setBotId(0); setView('bot') }
      else if (id === 'search') findIt()
      else if (['settings', 'inbox', 'chat', 'memory', 'skills', 'routines', 'activity', 'meetings'].includes(id)) setView(id as View)
    })).then(u => { off = u }, () => {})
    return () => off?.()
  }, [])
  // ⌘ on macOS, Ctrl elsewhere: 1–5 sections, 0 Needs you, comma Settings, N new bot, F search (this chat, or memory).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return
      const tab = TABS[Number(e.key) - 1]
      if (tab) setView(tab.id)
      else if (e.key === ',') setView('settings')
      else if (e.key === '0') setView('inbox')
      else if (e.key === 'n') { setBotId(0); setView('bot') }
      else if (e.key === 'f') findIt()
      else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  const chatName = (c: Chat) => c.chat_id === LOCAL ? LOCAL_NAME : c.title || `Chat ${c.chat_id}`
  const ch = CHANNEL[chat.chat_id === LOCAL ? 'local' : chat.channel] ?? CHANNEL.local

  return (
    <Bots.Provider value={botOf}>
    <Paused.Provider value={pausedOf}>
    <div className={'app' + (phoneList ? ' list' : '')}>
      <aside className="rail" aria-label="Bots and chats"
        onClick={e => { if ((e.target as Element).closest('.rail-item, .rail-label button')) setPhoneList(false) }}>
        <div className="rail-top" data-tauri-drag-region />
        <div className="rail-scroll" ref={railRef}>
          <span className="rail-glide" aria-hidden="true" />
          <div className="brand"><img src="/logo.png" alt="" />Mimir{remoteServer() && <span className="where" title={remoteServer()!.url}>server</span>}</div>
          <button className={'rail-item inbox-item' + (view === 'inbox' ? ' on' : '')} onClick={() => setView('inbox')}>
            <span className="ri-glyph"><Icon name="inbox" size={15} /></span>
            <span className="ri-text"><span className="ri-name">Needs you</span><span className="ri-sub">{needs ? 'Waiting for you' : 'All clear'}</span></span>
            {needs > 0 && <span className="badge count" key={needs}>{needs}</span>}
          </button>
          {inDesktop && (
            <button className={'rail-item' + (view === 'meetings' ? ' on' : '')} onClick={() => setView('meetings')}>
              <span className="ri-glyph"><Icon name="mic" size={15} /></span>
              <span className="ri-text"><span className="ri-name">Meetings</span><span className="ri-sub">{recording ? 'Transcribing…' : 'Transcribe a call'}</span></span>
              {recording && <span className="rec-dot" aria-label="Recording" />}
            </button>
          )}
          {computer.data?.enabled && (
            <button className={'rail-item' + (view === 'computer' ? ' on' : '')} onClick={() => setView('computer')}>
              <span className="ri-glyph"><Icon name="laptop" size={15} /></span>
              <span className="ri-text"><span className="ri-name">Computer</span><span className="ri-sub">{computer.data.running ? 'Watch or take over' : 'Starting…'}</span></span>
            </button>
          )}
          {allPause && (
            <button className="rail-item" onClick={() => setPause(null, false)} title="Resume all bots">
              <span className="ri-glyph"><Icon name="pause" size={15} /></span>
              <span className="ri-text"><span className="ri-name">All bots paused</span><span className="ri-sub">{allPause.until ? `Back ${ahead(allPause.until)}` : 'Click to resume'}</span></span>
            </button>
          )}
          <div className="rail-label">Bots<span>
            {!allPause && <button title="Pause all bots" aria-label="Pause all bots" onClick={e => { const r = e.currentTarget.getBoundingClientRect(); setMenu({ x: r.left, y: r.bottom + 4, id: null, pick: true }) }}><Icon name="pause" size={13} /></button>}
            <button title={`New bot  ${shortcut('N')}`} aria-label="New bot" onClick={() => openBot(0)}><Icon name="plus" size={14} /></button>
          </span></div>
          {botList.map(b => {
            const st = statusOf(b.id)
            const on = online(st)
            const p = presenceOf(st)
            return (
              <button key={b.id} className={'rail-item' + ((view === 'bot' && botId === b.id) || (botChat && (chatBot ?? botList[0]?.id) === b.id) ? ' on' : '')}
                onClick={() => talkTo(b.id)} title={`Chat with ${b.name} · right-click for settings`}
                onContextMenu={e => {
                  e.preventDefault()
                  // From the keyboard there's no pointer: open it by the row.
                  const r = e.currentTarget.getBoundingClientRect()
                  setMenu(e.clientX || e.clientY ? { x: e.clientX, y: e.clientY, id: b.id } : { x: r.left + 24, y: r.bottom, id: b.id })
                }}
                onKeyDown={e => {
                  // The menu key and Shift+F10 open the menu (not every browser turns them into a contextmenu event).
                  if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
                    e.preventDefault()
                    const r = e.currentTarget.getBoundingClientRect()
                    setMenu({ x: r.left + 24, y: r.bottom, id: b.id })
                  }
                }}>
                {(() => {
                  // Waiting on you beats working: the reply (or routine) can't go on until you answer.
                  const asks = approvals.some(a => a.bot_id === b.id)
                  const w: Busy | undefined = asks ? { bot_id: b.id, chat_id: 0, trigger: 'ask', step: 'Waiting for your OK' } : busy.data?.find(x => x.bot_id === b.id)
                  return <>
                    <Avatar id={b.id} name={b.name} presence={p} mood={w ? busyMood(w) : undefined} />
                    <span className="ri-text"><span className="ri-name">{b.name}</span>
                      <span className={'ri-sub' + (w ? ' busy' : '')}>{w ? busyLabel(w)
                        : pausedOf(b.id) !== undefined ? pausedText(pausedOf(b.id)!)
                        : on.length ? on.map(c => CHANNEL[c.kind].name).join(' · ') : p === 'err' ? 'Needs attention' : p === 'wait' ? 'Connecting…' : `${LOCAL_NAME} only`}</span></span>
                  </>
                })()}
              </button>
            )
          })}
          <div className="rail-label">Chats</div>
          {list.map(c => {
            const cc = CHANNEL[c.chat_id === LOCAL ? 'local' : c.channel] ?? CHANNEL.local
            return (
              <button key={c.chat_id} className={'rail-item' + (inChat && !botChat && c.chat_id === chat.chat_id ? ' on' : '')}
                onClick={() => { setChatId(c.chat_id); setViaBot(false); if (!inChat) setView(c.channel === 'household' ? 'memory' : 'chat') }}>
                <span className="ri-glyph"><Icon name={cc.icon} size={15} /></span>
                <span className="ri-text"><span className="ri-name">{chatName(c)}</span><span className="ri-sub">{c.family ? `${cc.name} · family` : `${cc.name}${c.guest ? ' · guest' : ''}${c.household && c.guest ? ' · household' : ''}`}</span></span>
                <span className="ri-count" title={`${c.memories} memories`}>{c.memories || ''}</span>
              </button>
            )
          })}
          {list.length <= 1 && <p className="rail-hint">Telegram, Slack and Discord chats appear here once you pair them.</p>}
        </div>
        <div className="rail-foot">
          <button className={'rail-item' + (view === 'settings' ? ' on' : '')} onClick={() => setView('settings')} title={`Settings  ${shortcut(',')}`}>
            <Icon name="settings" /> <span className="ri-text"><span className="ri-name">Settings</span></span>
          </button>
        </div>
      </aside>

      <main className="main" style={{ '--dx': `${dx}px` } as React.CSSProperties}>
        <header className="toolbar" data-tauri-drag-region>
          {inChat ? (
            <>
              <div className="toolbar-title" data-tauri-drag-region>
                <span className="ri-glyph" style={{ width: 26, height: 26, borderRadius: 7, display: 'grid', placeItems: 'center', background: 'var(--hover)', color: 'var(--text-2)' }}><Icon name={ch.icon} size={14} /></span>
                <span className="tt" data-tauri-drag-region><b>{botChat ? botOf(chatBot ?? botList[0]?.id ?? 1).name : chatName(chat)}</b><small>{ch.name} · {chat.memories} {chat.memories === 1 ? 'memory' : 'memories'}{household ? '' : ', shared by all your bots'}{chat.household && !household ? ' · reads the household' : ''}{!household && <GoalLine chatId={chat.chat_id} />}</small></span>
                {!botChat && chat.chat_id !== LOCAL && !household && !chat.family && (
                  <button className={'btn btn-sm btn-ghost' + (chat.guest ? ' guest-on' : '')}
                    title={chat.guest ? 'Paired by someone else: bots here have their own memory, and none of your email, calendar, meetings, saved logins or tool servers. Click if this chat is yours.' : 'Click if someone else uses this chat (a partner, a housemate): bots here then get none of your email, calendar, meetings, saved logins or tool servers.'}
                    onClick={() => ask(chat.guest ? 'Make this chat yours? Bots here get your email, calendar, meetings, saved logins and tool servers.' : 'Treat this chat as a guest’s? Bots here lose your email, calendar, meetings, saved logins and tool servers.', chat.guest ? 'Make it mine' : 'Make it a guest’s')
                      .then(ok => { if (ok) void invoke('chat_guest', { chatId: chat.chat_id, guest: !chat.guest }).then(chats.reload, e => tell(String(e))) })}>
                    {chat.guest ? 'Guest' : 'Yours'}
                  </button>
                )}
                {/* A guest's own chat can share the household's lists; a group can become one the whole household talks in. */}
                {!botChat && chat.chat_id !== LOCAL && !household && chat.private && chat.guest && (
                  <button className={'btn btn-sm btn-ghost' + (chat.household ? ' guest-on' : '')}
                    title={chat.household ? 'In your household: bots here read the household’s memories, and what this guest adds comes to you as a suggestion. Click to take them out.' : 'Add this guest to your household: bots here read the household’s memories and lists, and what they add comes to you as a suggestion first.'}
                    onClick={() => void invoke('chat_household', { chatId: chat.chat_id, on: !chat.household }).then(chats.reload, e => tell(String(e)))}>
                    <Icon name="home" size={13} /> {chat.household ? 'In household' : 'Add to household'}
                  </button>
                )}
                {!botChat && chat.chat_id !== LOCAL && !household && chat.private === false && (
                  <button className={'btn btn-sm btn-ghost' + (chat.family ? ' guest-on' : '')}
                    title={chat.family ? 'A family group: everyone in your household can talk to the bots here. Like a guest’s chat, bots here get none of your email, calendar, meetings, saved logins or tool servers.' : 'Make this a family group: everyone in your household can talk to the bots here, and they read the household’s memories.'}
                    onClick={() => (chat.family ? Promise.resolve(true) : ask('Make this a family group? Everyone in your household can talk to the bots here, and bots here lose your email, calendar, meetings, saved logins and tool servers (like a guest’s chat).', 'Make it a family group'))
                      .then(ok => { if (ok) void invoke('chat_family', { chatId: chat.chat_id, on: !chat.family }).then(chats.reload, e => tell(String(e))) })}>
                    <Icon name="home" size={13} /> {chat.family ? 'Family group' : 'Make family group'}
                  </button>
                )}
              </div>
              <Segmented label="Sections" value={view as Tab} onChange={setView} options={household ? TABS.filter(t => t.id === 'memory') : TABS} />
            </>
          ) : (
            <div className="toolbar-title" data-tauri-drag-region>
              <span className="tt" data-tauri-drag-region><b>{view === 'settings' ? 'Settings' : view === 'inbox' ? 'Needs you' : view === 'computer' ? 'Computer' : view === 'meetings' ? 'Meetings' : botId === 0 ? 'New bot' : botOf(botId).name}</b></span>
            </div>
          )}
        </header>

        {view !== 'inbox' && <Approvals list={approvals} chats={list} chatName={chatName} onDecided={inbox.reload} />}
        {signIn && <SignInForm onClose={() => { setSignIn(false); inbox.reload() }} />}

        {view === 'chat' ? (
          <ChatView key={`chat-${chat.chat_id}-${incognito && chat.chat_id === LOCAL}-${botChat ? chatBot ?? botList[0]?.id : 'all'}`} only={botChat} chat={chat} bots={botList} onTurn={chats.reload}
            incognito={incognito && chat.chat_id === LOCAL} setIncognito={setIncognito} openBot={openBot} openSettings={openSettings}
            botId={chatBot ?? undefined} onBot={setChatBot} troubleOf={id => troubleOf(statusOf(id))} busy={busy.data ?? undefined} />
        ) : (
          // Keyed: each view starts at the top (the scroller used to keep the last view's position).
          <div className="page" key={`${view}-${chat.chat_id}-${botId}`}>
            <div className={'col fade-in' + (view === 'activity' || view === 'computer' ? ' wide' : '')}>
              {view === 'memory' && <MemoryView chatId={chat.chat_id} onChange={chats.reload} />}
              {view === 'skills' && <SkillsView chatId={chat.chat_id} />}
              {view === 'routines' && <RoutinesView chatId={chat.chat_id} bots={botList} />}
              {view === 'activity' && <ActivityView chatId={chat.chat_id} />}
              {view === 'settings' && <SettingsView onComputer={computer.reload} find={find} />}
              {view === 'computer' && <ComputerView bots={botList} chats={list} chatName={chatName} />}
              {view === 'meetings' && <MeetingsView bots={botList} onSent={talkTo} />}
              {view === 'inbox' && <InboxView data={inbox.data} troubled={troubled} reload={inbox.reload} chats={list} chatName={chatName} openBot={openBot} openChat={openChat} openSettings={() => setView('settings')} />}
              {view === 'bot' && (
                // Keyed on whether the bot has loaded too: right after "Create bot" the list hasn't caught up yet.
                <BotPage key={`${botId}:${botList.some(b => b.id === botId)}`} bot={botList.find(b => b.id === botId)} status={statusOf(botId)} canDelete={botList.length > 1}
                  // Only a new bot's page moves (to the bot it made): a slow save mustn't pull you back from another bot.
                  onSaved={id => { bots.reload(); statuses.reload(); if (botId === 0) setBotId(id) }}
                  onDeleted={() => { bots.reload(); if (chatBot === botId) { setChatBot(null); setViaBot(false) } setView('chat') }} onChat={() => talkTo(botId)} onImport={text => setImporting({ text })} />
              )}
            </div>
          </div>
        )}
      </main>
      <nav className="tabbar" aria-label="Sections">
        {([['chats', 'Chats', 'chat'], ['inbox', 'Needs you', 'inbox'], ['memory', 'Memory', 'memory'], ['routines', 'Routines', 'clock']] as const).map(([id, label, icon]) => (
          <button key={id} className={phoneTab === id ? 'on' : ''} aria-current={phoneTab === id ? 'page' : undefined}
            onClick={() => id === 'chats' ? setPhoneList(true) : go(id)}>
            <Icon name={icon} size={20} />{label}
            {id === 'inbox' && needs > 0 && <span className="badge count">{needs}</span>}
          </button>
        ))}
      </nav>
      {menu && (menu.pick || menu.id === null
        ? <Menu key="pick" at={menu} name={menu.id === null ? 'Pause all bots' : `Pause ${botOf(menu.id).name}`} items={pauseChoices(until => setPause(menu.id, true, until))} onClose={() => setMenu(null)} />
        : (() => {
          const id = menu.id, name = botOf(id).name
          return <Menu key="bot" at={menu} name={name} onClose={() => setMenu(null)} items={[
            ['chat', `Chat with ${name}`, () => talkTo(id)],
            ['settings', 'Settings', () => openBot(id)],
            ['copy', 'Duplicate…', () => { setMenu(null); invoke('export_bot', { id }).then(f => setImporting({ text: JSON.stringify(f), duplicateOf: id }), e => tell(String(e))) }],
            ['archive', 'Export…', () => {
              setMenu(null)
              invoke('export_bot', { id }).then(f => saveDownload(`${name.replace(/[^\p{L}\p{N}]+/gu, '-')}.mimir-bot.json`, JSON.stringify(f, null, 2)))
                .then(where => tell(`Saved to ${where}. It has no tokens, keys, memories or chats. Check the role and tool server settings for passwords before you share this file.`), e => tell(String(e)))
            }],
            ownPause(id) ? ['play', `Resume ${name}`, () => setPause(id, false)] : ['pause', `Pause ${name}…`, () => setMenu({ ...menu, pick: true })],
          ]} />
        })())}
      {importing && <BotImport text={importing.text} duplicateOf={importing.duplicateOf} onClose={() => setImporting(null)} onAdded={id => { setImporting(null); bots.reload(); statuses.reload(); openBot(id) }} />}
    </div>
    </Paused.Provider>
    </Bots.Provider>
  )
}
