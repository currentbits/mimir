import { invoke as tauriInvoke } from '@tauri-apps/api/core'

/** Running inside the desktop app (vs. a browser on a self-hosted server). */
export const inDesktop = '__TAURI_INTERNALS__' in window

/** The OS the UI runs on (the desktop app's, or the browser's for the self-hosted web UI). */
export const platform: 'mac' | 'win' | 'linux' = /Mac|iPhone|iPad/.test(navigator.userAgent) ? 'mac' : /Windows/.test(navigator.userAgent) ? 'win' : 'linux'
/** A shortcut label in the platform's own notation: ⌘1 on a Mac, Ctrl+1 elsewhere. */
export const shortcut = (key: string) => (platform === 'mac' ? `⌘${key}` : `Ctrl+${key}`)
/** Where the desktop app keeps secrets on this OS. */
export const keystore = { mac: 'the macOS Keychain', win: 'Windows Credential Manager', linux: 'your system keyring' }[platform]
/** Where the app lives while its window is closed. */
export const trayName = platform === 'mac' ? 'menu bar' : 'system tray'

export type Server = { url: string; token: string }
const SERVER_KEY = 'mimir.server'
const TOKEN_KEY = 'mimir.token'

/** Desktop pointed at a self-hosted server, if any (its address; the token is in the keychain). */
export function remoteServer(): { url: string } | null {
  try { const s = JSON.parse(localStorage.getItem(SERVER_KEY) ?? 'null'); return s?.url ? { url: s.url } : null } catch { return null }
}

/** The connected server's admin token, read from the keychain once. A token saved by an older version (in the
 *  window's storage) moves to the keychain on first use. */
let remoteToken: Promise<string> | null = null
function serverToken(): Promise<string> {
  remoteToken ??= (async () => {
    const old = (() => { try { return JSON.parse(localStorage.getItem(SERVER_KEY) ?? 'null') } catch { return null } })()
    if (old?.token) {
      await tauriInvoke('api', { cmd: 'remote_token', args: { set: old.token } })
      localStorage.setItem(SERVER_KEY, JSON.stringify({ url: old.url }))
      return old.token as string
    }
    return (await tauriInvoke<string | null>('api', { cmd: 'remote_token', args: {} })) ?? ''
  })()
  return remoteToken
}

/** Plain http sends the admin token in the clear: fine at home or over Tailscale, not across the internet. */
export function insecureServer(url: string): string | null {
  let u: URL
  try { u = new URL(url) } catch { return 'That isn’t a web address (it should start with https://).' }
  if (u.protocol === 'https:') return null
  if (u.protocol !== 'http:') return 'The address should start with https://.'
  const h = u.hostname.replace(/^\[|\]$/g, '')
  // A whole IPv4 address, not just a name that starts with digits ("10.example.com").
  const ip = /^\d{1,3}(\.\d{1,3}){3}$/.test(h)
  const near = h === 'localhost' || h === '::1' || /\.(local|lan|home|internal|ts\.net)$/.test(h)
    || (ip && /^(127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.)/.test(h))
  return near ? null : 'Use https:// for a server on the internet: over http the admin token travels in the clear.'
}

/** Browser on the server itself: just the admin token, same origin. */
export const webToken = () => localStorage.getItem(TOKEN_KEY)
export const setWebToken = (t: string | null) => t ? localStorage.setItem(TOKEN_KEY, t) : localStorage.removeItem(TOKEN_KEY)

const snake = (args: Record<string, unknown>) =>
  Object.fromEntries(Object.entries(args).map(([k, v]) => [k.replace(/[A-Z]/g, c => '_' + c.toLowerCase()), v]))

async function post<T>(base: string, token: string | null, cmd: string, args: Record<string, unknown>): Promise<T> {
  const r = await fetch(`${base.replace(/\/$/, '')}/api/${cmd}`, {
    method: 'POST',
    // The token goes to this address only, never along a redirect to another.
    redirect: 'error',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token ?? ''}` },
    body: JSON.stringify(snake(args)),
  })
  const body = await r.json().catch(() => ({ error: `${r.status} ${r.statusText}` }))
  if (r.status === 401) window.dispatchEvent(new Event('mimir-unauthorized'))
  if (!r.ok) throw body.error ?? `HTTP ${r.status}`
  return body as T
}

/** Every UI command: in-process on desktop, or the server's /api over HTTPS. */
export function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const remote = remoteServer()
  if (inDesktop && !remote) return tauriInvoke<T>('api', { cmd, args: snake(args) })
  return remote ? serverToken().then(t => post<T>(remote.url, t, cmd, args)) : post<T>('', webToken(), cmd, args)
}

/** Desktop only: something this computer does itself even when connected to a server (recording a meeting). */
export const localInvoke = <T>(cmd: string, args: Record<string, unknown> = {}) => tauriInvoke<T>('api', { cmd, args: snake(args) })

/** Check a server + token before saving them. */
export const probe = (url: string, token: string) => post<{ mode: string; version: string }>(url, token, 'info', {})

/** Desktop only: point the app at a server (or back to this computer) and stop/start local bots. */
export async function connectServer(s: Server | null) {
  await tauriInvoke('api', { cmd: 'remote_token', args: { set: s?.token ?? '' } })
  await tauriInvoke('api', { cmd: 'local_bots', args: { enabled: !s } })
  if (s) localStorage.setItem(SERVER_KEY, JSON.stringify({ url: s.url }))
  else localStorage.removeItem(SERVER_KEY)
  location.reload()
}

/** Copy text. The desktop app copies itself: its webview refuses clipboard writes after a wait, and sometimes at all. */
export async function copyText(text: string | Promise<string>): Promise<void> {
  if (inDesktop) return localInvoke('copy_text', { text: await text })
  if (typeof text !== 'string' && typeof ClipboardItem !== 'undefined') {
    // Safari keeps the click's permission only if the copy starts now, with the text still to come.
    return navigator.clipboard.write([new ClipboardItem({ 'text/plain': text.then(t => new Blob([t], { type: 'text/plain' })) })])
  }
  return navigator.clipboard.writeText(await text)
}

/** Save a file to Downloads; says where it went. (The desktop app's webview downloads nothing by itself.) */
export async function saveDownload(name: string, text: string, type = 'application/json'): Promise<string> {
  if (inDesktop) return localInvoke<string>('save_download', { name, text })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(new Blob([text], { type }))
  a.download = name
  a.click()
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
  return 'your downloads'
}
