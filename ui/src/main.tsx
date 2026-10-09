import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { inDesktop, invoke, localInvoke, platform } from './api'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { QuickAsk } from './quick'

/** The desktop app has two windows: the main one and the quick-ask panel. */
const isQuick = inDesktop && (() => { try { return getCurrentWindow().label === 'quick' } catch { return false } })()
if (isQuick) document.documentElement.dataset.window = 'quick'

// Desktop shell per OS. macOS: transparent window, sidebar vibrancy, inset traffic lights
// (tauri.macos.conf.json). Windows and Linux: the native frame, opaque surfaces, slim scrollbars.
if (inDesktop) document.documentElement.dataset.shell = platform

// A window in the background goes quiet like a native one: selection greys, switches lose their colour.
if (inDesktop) {
  const mark = () => document.documentElement.toggleAttribute('data-inactive', !document.hasFocus())
  window.addEventListener('focus', mark)
  window.addEventListener('blur', mark)
  mark()
}

// Desktop smoke test (MIMIR_SELFTEST=1): report what rendered through the real IPC, then the app exits.
const errors: string[] = []
window.addEventListener('error', e => errors.push(String(e.message)))
window.addEventListener('unhandledrejection', e => errors.push(String(e.reason)))
if (inDesktop && !isQuick) {
  localInvoke<boolean>('selftest').then(on => on && setTimeout(async () => {
    const accounts = await invoke('accounts').catch(e => String(e))
    const shortcut = await invoke('quick_shortcut').catch(e => String(e))
    invoke('selftest_report', { text: document.body.innerText.slice(0, 4000), errors, accounts, shortcut })
  }, 5000))
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isQuick ? <QuickAsk /> : <App />}
  </StrictMode>,
)
