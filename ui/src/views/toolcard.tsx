import { useEffect, useRef, useState } from 'react'
import { inDesktop, invoke, localInvoke, platform, remoteServer } from '../api'
import { ask, tell } from '../ui'

/** A tool server's interactive card (MCP Apps), as saved with the reply (db/tool_cards.rs). */
export type ToolCardData = { key: string; tool: string; input: unknown; result: unknown }

/** Where the card's page is served, with its own CSP: the desktop app's mimir-card:, or the server's /card/<key>. */
function cardUrl(key: string) {
  const remote = remoteServer()
  if (inDesktop && !remote) return platform === 'win' ? `http://mimir-card.localhost/${key}` : `mimir-card://localhost/${key}`
  return `${remote ? remote.url.replace(/\/$/, '') : ''}/card/${key}`
}

const MAX_HEIGHT = 640

/** The card in a sandboxed frame (scripts only: no same origin, no top navigation, no popups), speaking MCP Apps'
 *  postMessage protocol. Its buttons call its own server's tools through the tool gate (card_call): a write asks like
 *  any other. A message it wants to send lands in the composer, for you to send or not. */
export function ToolCard({ card, onMessage }: { card: ToolCardData; onMessage: (text: string) => void }) {
  const frame = useRef<HTMLIFrameElement>(null)
  const [height, setHeight] = useState(160)
  const [server, tool] = card.tool.split('__')
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      const w = frame.current?.contentWindow
      const m = e.data
      if (!w || e.source !== w || m?.jsonrpc !== '2.0' || typeof m.method !== 'string') return
      // Its origin is opaque ('null'), so it can't be named as the target.
      const post = (msg: object) => w.postMessage({ jsonrpc: '2.0', ...msg }, '*')
      const reply = (result: unknown) => post({ id: m.id, result })
      const fail = (code: number, message: string) => m.id !== undefined && post({ id: m.id, error: { code, message } })
      const p = m.params ?? {}
      switch (m.method) {
        case 'ui/initialize':
          return reply({
            protocolVersion: p.protocolVersion ?? '2026-01-26',
            hostInfo: { name: 'mimir', version: '1' },
            hostCapabilities: { openLinks: {}, serverTools: {} },
            hostContext: {
              theme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
              displayMode: 'inline', availableDisplayModes: ['inline'], platform: inDesktop ? 'desktop' : 'web',
              locale: navigator.language, timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
              containerDimensions: { maxHeight: MAX_HEIGHT },
              toolInfo: { tool: { name: tool } },
            },
          })
        case 'ui/notifications/initialized':
          post({ method: 'ui/notifications/tool-input', params: { arguments: card.input ?? {} } })
          return post({ method: 'ui/notifications/tool-result', params: card.result ?? {} })
        case 'ui/notifications/size-changed':
          if (typeof p.height === 'number') setHeight(Math.max(40, Math.min(MAX_HEIGHT, Math.ceil(p.height))))
          return
        case 'notifications/message':
          return
        case 'tools/call':
          if (typeof p.name !== 'string') return fail(-32602, 'name is missing')
          return void invoke('card_call', { key: card.key, name: p.name, arguments: p.arguments ?? {} }).then(reply, err => fail(-32603, String(err)))
        case 'ui/message': {
          const text = ([] as { type?: string; text?: string }[]).concat(p.content ?? []).filter(c => c?.type === 'text').map(c => c.text).join('\n').trim()
          if (text) onMessage(text)
          return reply({})
        }
        case 'ui/open-link': {
          const url = String(p.url ?? '')
          if (!/^https?:\/\//.test(url)) return fail(-32602, 'only web links open')
          return void ask(`${server}'s card wants to open ${url}`, 'Open').then(ok => {
            if (!ok) return fail(-32000, 'the person said no')
            if (inDesktop) localInvoke('open_url', { url }).catch(err => tell(`Couldn't open the link: ${err}`))
            else window.open(url, '_blank', 'noopener,noreferrer')
            reply({})
          })
        }
        default:
          return fail(-32601, `${m.method} isn't supported here`)
      }
    }
    addEventListener('message', onMsg)
    return () => removeEventListener('message', onMsg)
  }, [card, server, tool, onMessage])
  return (
    <div className="tool-card">
      <iframe ref={frame} src={cardUrl(card.key)} sandbox="allow-scripts" referrerPolicy="no-referrer" title={`${server} · ${tool}`} style={{ height }} />
      <small>{server} · {tool}</small>
    </div>
  )
}
