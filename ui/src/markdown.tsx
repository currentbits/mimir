// A small Markdown renderer for bot replies. Builds React elements (never raw HTML), so a reply
// can't inject markup. Covers what chat models actually write: paragraphs, lists, headings,
// quotes, code, bold/italic, links.
import { tell } from './ui'
import { memo, type ReactNode } from 'react'
import { inDesktop, localInvoke } from './api'

type Block =
  | { kind: 'p' | 'h' | 'quote'; text: string }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'code'; text: string }

const BULLET = /^\s*[-*•]\s+(.*)$/
const NUMBER = /^\s*\d+[.)]\s+(.*)$/

function blocks(md: string): Block[] {
  const lines = md.replace(/\r/g, '').split('\n')
  const out: Block[] = []
  let para: string[] = []
  const flush = () => { if (para.length) { out.push({ kind: 'p', text: para.join('\n') }); para = [] } }
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (line.trim().startsWith('```')) {
      flush()
      const code: string[] = []
      while (++i < lines.length && !lines[i].trim().startsWith('```')) code.push(lines[i])
      out.push({ kind: 'code', text: code.join('\n') })
      continue
    }
    const h = line.match(/^#{1,6}\s+(.*)$/)
    const list = line.match(BULLET) ? 'ul' : line.match(NUMBER) ? 'ol' : null
    if (h) { flush(); out.push({ kind: 'h', text: h[1] }) }
    else if (list) {
      flush()
      const re = list === 'ul' ? BULLET : NUMBER
      const items: string[] = []
      for (; i < lines.length; i++) {
        const m = lines[i].match(re)
        if (m) items.push(m[1])
        else if (/^\s{2,}\S/.test(lines[i]) && items.length) items[items.length - 1] += ' ' + lines[i].trim()
        else { i--; break }
      }
      out.push({ kind: list, items })
    }
    else if (/^>\s?/.test(line)) { flush(); out.push({ kind: 'quote', text: line.replace(/^>\s?/, '') }) }
    else if (!line.trim()) flush()
    else para.push(line)
  }
  flush()
  return out
}

/** Open links in the user's browser, never inside the app window. */
export function openLink(e: React.MouseEvent<HTMLAnchorElement>) {
  e.preventDefault()
  const url = e.currentTarget.href
  if (inDesktop) localInvoke('open_url', { url }).catch(e => tell(`Couldn't open the link: ${e}`))
  else window.open(url, '_blank', 'noopener,noreferrer')
}

const INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(\*[^*\s][^*\n]*\*)|((?<![\w])_[^_\s][^_\n]*_(?![\w]))|(\[[^\]\n]+\]\(https?:\/\/[^\s)]+\))|(https?:\/\/[^\s<>]*[^\s<>.,;:!?)\]'"])/g

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = []
  let last = 0
  let k = 0
  for (const m of text.matchAll(INLINE)) {
    const t = m[0]
    if (m.index! > last) out.push(...breaks(text.slice(last, m.index), k++))
    if (m[1]) out.push(<code key={k++}>{t.slice(1, -1)}</code>)
    else if (m[2]) out.push(<strong key={k++}>{inline(t.slice(2, -2))}</strong>)
    else if (m[3] || m[4]) out.push(<em key={k++}>{inline(t.slice(1, -1))}</em>)
    else if (m[5]) {
      const [, label, href] = t.match(/^\[([^\]]+)\]\((.+)\)$/)!
      // Every labelled link says where it really goes (as the browser reads the address: look-alike letters show as
      // xn--…), unless the label is just that address.
      const target = (() => { try { return new URL(href).hostname.toLowerCase().replace(/^www\./, '') } catch { return '' } })()
      const plain = label.replace(/[*_`]/g, '').trim().toLowerCase().replace(/^https?:\/\//, '').replace(/^www\./, '').replace(/\/.*$/, '')
      out.push(<a key={k++} href={href} onClick={openLink} title={href}>{inline(label)}{target && plain !== target && <small className="link-real"> ({target})</small>}</a>)
    } else out.push(<a key={k++} href={t} onClick={openLink}>{t.replace(/^https?:\/\//, '')}</a>)
    last = m.index! + t.length
  }
  if (last < text.length) out.push(...breaks(text.slice(last), k++))
  return out
}

/** Single newlines inside a paragraph stay line breaks. */
function breaks(s: string, key: number): ReactNode[] {
  return s.split('\n').flatMap((part, i) => i ? [<br key={`${key}-${i}`} />, part] : [part])
}

// memo: a chat redraws on every poll, and its old replies haven't changed.
export const Markdown = memo(function Markdown({ text, caret }: { text: string; caret?: boolean }) {
  const bs = blocks(text)
  return (
    <div className="md">
      {bs.map((b, i) => {
        const end = caret && i === bs.length - 1 ? <span className="caret" /> : null
        switch (b.kind) {
          case 'h': return <p key={i} className="md-h">{inline(b.text)}{end}</p>
          case 'quote': return <blockquote key={i}>{inline(b.text)}{end}</blockquote>
          case 'code': return <pre key={i}><code>{b.text}</code>{end}</pre>
          case 'ul': return <ul key={i}>{b.items.map((t, j) => <li key={j}>{inline(t)}{j === b.items.length - 1 && end}</li>)}</ul>
          case 'ol': return <ol key={i}>{b.items.map((t, j) => <li key={j}>{inline(t)}{j === b.items.length - 1 && end}</li>)}</ol>
          default: return <p key={i}>{inline(b.text)}{end}</p>
        }
      })}
      {caret && bs.length === 0 && <span className="caret" />}
    </div>
  )
})
