// The app's main flows clicked through in a browser, switching bots mid-reply too (npm i playwright; npx playwright install chromium), against a
// server set up as for scripts/e2e-flows.py, with MIMIR_UI_DIR=ui/dist: node scripts/ui-flows.mjs
import pkg from 'playwright'; const { chromium } = pkg
import { execFileSync } from 'node:child_process'
// Ada and Search on the fake AI (scripts/fake-ai.py on :8899), in a fresh server.
const api = (cmd, args = {}) => fetch(`http://127.0.0.1:8789/api/${cmd}`, { method: 'POST', headers: { Authorization: 'Bearer testtoken-abcdefghijklmnop', 'Content-Type': 'application/json' }, body: JSON.stringify(args) }).then(r => r.json())
const fake = { provider: 'openai', model: 'fake', base_url: 'http://127.0.0.1:8899/v1', tools: '', look: '' }
await api('onboarding_done'); await api('set_api_key', { kind: 'openai', key: 'k' })
await api('save_bot', { bot: { id: 1, name: 'Ada', persona: '', ...fake } })
if (!(await api('bots')).some(x => x.name === 'Search')) await api('save_bot', { bot: { id: 0, name: 'Search', persona: 'SEO research', ...fake } })
const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 1280, height: 860 }, colorScheme: 'dark' })
const errs = []; p.on('pageerror', e => errs.push('pageerror: ' + e.message)); p.on('console', m => m.type() === 'error' && errs.push('console: ' + m.text()))
p.on('dialog', d => { errs.push('NATIVE DIALOG: ' + d.message()); d.dismiss() })
await p.addInitScript(() => { localStorage.setItem('mimir.token', 'testtoken-abcdefghijklmnop'); localStorage.setItem('mimir.onboarded', '1') })
let failed = 0
const check = (name, ok, detail = '') => { if (!ok) failed++; console.log((ok ? 'PASS ' : 'FAIL ') + name + (!ok && detail ? `  [${detail}]` : '')) }
const main = () => p.locator('.main').innerText()
const bot = name => p.locator('.rail-item', { hasText: name }).filter({ hasNot: p.locator('text=Built-in chat') }).first()
const tab = name => p.locator('.toolbar').getByText(name, { exact: true }).first()
const say = async text => { const box = p.locator('textarea').first(); await box.fill(text); await box.press('Enter') }
const until = async (fn, ms = 15000) => { const t = Date.now(); while (Date.now() - t < ms) { if (await fn()) return true; await p.waitForTimeout(300) } return false }

await p.goto('http://127.0.0.1:8789/'); await p.waitForTimeout(1500)

// Chat with Ada, then Search: each its own thread
await bot('Ada').click(); await p.waitForTimeout(500)
await say('hello Ada')
check('Ada replies in the app', await until(async () => (await main()).includes('OK: hello Ada')))
await bot('Search').click(); await p.waitForTimeout(800)
check("Search's thread doesn't show Ada's", !(await main()).includes('OK: hello Ada'))
await say('check the title')
check('Search replies', await until(async () => (await main()).includes('SEARCH-ANSWER')))
await bot('Ada').click(); await p.waitForTimeout(800)
check("Ada's thread is back, without Search's", (await main()).includes('OK: hello Ada') && !(await main()).includes('SEARCH-ANSWER: the title tag is fine (asked: check the title)'))

// "@Search …" from Ada's own chat: your message stays on screen while Search answers, then Search's chat opens
await say('@Search SLOW about meta tags'); await p.waitForTimeout(700)
check('your @mention message stays while the reply is written', (await main()).includes('SLOW about meta tags'))
check("Search's own chat opens with the answer", await until(async () => (await main()).includes('SEARCH-ANSWER: the title tag is fine (asked: SLOW about meta tags)')
  && (await p.locator('.composer textarea').getAttribute('placeholder'))?.includes('Search')))

// Leaving mid-reply and coming back: the reply still shows as it's written, and arrives
await bot('Ada').click(); await p.waitForTimeout(500)
await say('SLOW are you there'); await p.waitForTimeout(500)
await bot('Search').click(); await p.waitForTimeout(600)
await bot('Ada').click(); await p.waitForTimeout(1800)
check('a reply in progress shows after coming back', (await p.locator('.msg-bot .typing, .msg-bot .steps').count()) > 0 && await p.locator('.composer textarea').isDisabled())
check('and it arrives', await until(async () => (await main()).includes('OK: SLOW are you there')))
check('the composer unlocks after it', await until(async () => !(await p.locator('.composer textarea').isDisabled()), 5000))

// Delegation from the app
await say('please ASK SEARCH')
check('delegation answer shows', await until(async () => (await main()).includes('DONE ask_bot')))

// An approval card in the app: MAKE BOT → Allow
await say('MAKE BOT for prices')
check('approval card appears', await until(async () => (await p.locator('.approval').count()) > 0))
const allow = p.locator('.approval').getByRole('button', { name: /Allow|Approve|Make/i }).first()
await allow.click().catch(e => errs.push('allow: ' + e.message))
check('bot made after approving', await until(async () => (await p.locator('.rail-item', { hasText: 'Scout' }).count()) > 0, 20000))

// Routines: create by chat, pause, resume, run now, edit, delete
await say('set a ROUTINE')
await until(async () => (await main()).includes('DONE routine_create'))
await tab('Routines').click(); await p.waitForTimeout(1000)
check('routine listed', (await p.locator('.routine').count()) === 1)
const sw = p.locator('.routine [role=switch], .routine input[type=checkbox]').first()
await sw.click(); await p.waitForTimeout(800)
check('pause shows Paused', (await p.locator('.routine').innerText()).includes('Paused'))
await sw.click(); await p.waitForTimeout(800)
check('resume clears Paused', !(await p.locator('.routine').innerText()).includes('Paused'))
await p.locator('.routine').getByRole('button', { name: 'Run now' }).click()
check('run now finishes', await until(async () => (await p.locator('.routine').getByRole('button', { name: 'Run now' }).count()) > 0 && (await p.locator('.rh-dots .dot').count()) > 0, 20000))
await p.locator('.routine').getByRole('button', { name: 'Edit' }).click(); await p.waitForTimeout(400)
const prompt = p.locator('.routine-inline textarea').first()
await prompt.fill('Brief me on tech news, edited'); await p.locator('.routine-inline').getByRole('button', { name: /Save/ }).click(); await p.waitForTimeout(800)
check('edit saved', (await main()).includes('edited'))
await p.getByRole('button', { name: 'Delete routine' }).click(); await p.waitForTimeout(200)
await p.locator('.ask-box').getByRole('button', { name: 'Delete' }).click(); await p.waitForTimeout(800)
check('routine deleted', (await p.locator('.routine').count()) === 0)

// Memory: remember by chat, then forget and erase from the Memory tab
await tab('Chat').click(); await p.waitForTimeout(500)
await say('REMEMBER my dentist')
await until(async () => (await main()).includes('DONE memory_store'))
await tab('Memory').click(); await p.waitForTimeout(1000)
check('memory listed', (await main()).includes('Dr. Lee in Stellenbosch'))
const forgetBtn = p.getByRole('button', { name: /^Forget Dentist$/ }).first()
check('Forget button there', (await forgetBtn.count()) > 0)
if (await forgetBtn.count()) { await forgetBtn.click(); await p.waitForTimeout(400); if (await p.locator('.ask-box').count()) await p.locator('.ask-box button.btn-primary, .ask-box button.btn-danger-solid').click(); await p.waitForTimeout(800) }
check('forgotten memory leaves the list', await until(async () => (await p.getByRole('button', { name: /^Forget Dentist$/ }).count()) === 0, 5000))

// Skills: add one from the library
await tab('Skills').click(); await p.waitForTimeout(1200)
const add = p.getByRole('button', { name: /^Add/ }).first()
check('library Add button', (await add.count()) > 0)
if (await add.count()) { await add.click(); await p.waitForTimeout(1200) }
check('skill added', (await main()).match(/Morning briefing|Inbox triage|added/i) !== null)

// Activity has the runs
await tab('Activity').click(); await p.waitForTimeout(1200)
check('activity lists runs', (await p.locator('.run').count()) > 3)

// Delete the new bot from its page
await p.locator('.rail-item', { hasText: 'Scout' }).first().click({ button: 'right' }); await p.waitForTimeout(300)
await p.getByRole('menuitem', { name: 'Settings' }).click(); await p.waitForTimeout(800)
await p.getByRole('button', { name: 'Delete bot' }).click(); await p.waitForTimeout(200)
await p.locator('.ask-box').getByRole('button', { name: 'Delete bot' }).click(); await p.waitForTimeout(1200)
check('bot deleted', (await p.locator('.rail-item', { hasText: 'Scout' }).count()) === 0)

const settings = async name => { await bot(name).click({ button: 'right' }); await p.waitForTimeout(300); await p.getByRole('menuitem', { name: 'Settings' }).click(); await p.waitForTimeout(800) }

// A slow reply, then switch away at once: it doesn't land in the other bot's thread, and is there on return
await bot('Ada').click(); await p.waitForTimeout(500)
await say('SLOW one')
await p.waitForTimeout(300)
await bot('Search').click(); await p.waitForTimeout(500)
check("Search's thread doesn't show Ada's pending message", !(await main()).includes('SLOW one'), await main())
await p.waitForTimeout(5000)
check("Search's thread doesn't show Ada's late reply", !(await main()).includes('OK: SLOW one'))
const composer = await p.locator('textarea').first().isEnabled()
check("Search's composer is usable", composer)
await bot('Ada').click()
check("Ada's reply is there on return", await until(async () => (await main()).includes('OK: SLOW one'), 4000), await main())

// Switch away and back mid-reply: the thread shows it's working, then the reply
await say('SLOW two'); await p.waitForTimeout(300)
await bot('Search').click(); await p.waitForTimeout(400); await bot('Ada').click(); await p.waitForTimeout(800)
check('back mid-reply: the reply in progress shows', (await p.locator('.thread [aria-label="Thinking"], .thread .steps').count()) > 0)
check('back mid-reply: the reply arrives without a reload', await until(async () => (await main()).includes('OK: SLOW two'), 8000))

// Draft typed for Ada, switch to Search: Search's composer is empty
await p.locator('textarea').first().fill('draft for ada')
await bot('Search').click(); await p.waitForTimeout(500)
check("Ada's draft doesn't show in Search's composer", (await p.locator('textarea').first().inputValue()) !== 'draft for ada')
await bot('Ada').click(); await p.waitForTimeout(500)

// Bot settings: Ada's page, then Search's: Search's own name and role
await settings('Ada')
const nameOf = () => p.locator('.main input').first().inputValue()
check("Ada's page shows Ada", (await nameOf()) === 'Ada', await nameOf())
await settings('Search')
check("Search's page shows Search", (await nameOf()) === 'Search', await nameOf())
check("Search's page shows its role", (await main()).includes('SEO research') || (await p.locator('.main textarea').first().inputValue()).includes('SEO research'))
// Rename from the page
await p.locator('.main input').first().fill('Searcher')
const save = p.locator('.main').getByRole('button', { name: /^Save/ }).first()
check('bot page has a Save button', (await save.count()) > 0)
if (await save.count()) { await save.click(); await p.waitForTimeout(1200) }
check('rename shows in the sidebar', (await p.locator('.rail-item', { hasText: 'Searcher' }).count()) > 0)

// "@Name" from a bot's own thread moves to that bot's thread, with the message on its way
await bot('Ada').click(); await p.waitForTimeout(500)
await say('@Ada SLOW three'); await p.waitForTimeout(600)
check('@mention: the message shows at once', (await main()).includes('SLOW three'))
check('@mention: the reply arrives', await until(async () => (await main()).includes('OK: SLOW three'), 8000))

// Every tab while on Ada, then switch chat to Search with each tab open: no errors
for (const t of ['Memory', 'Skills', 'Routines', 'Activity', 'Chat']) {
  await bot('Ada').click(); await p.waitForTimeout(300)
  if (await tab(t).count()) { await tab(t).click(); await p.waitForTimeout(500) }
  await bot('Searcher').click(); await p.waitForTimeout(500)
}
// A chat past the 200 messages the UI loads (issue #7): replies alternate a lookup (no chip) and a saved memory (a chip),
// so each exchange shifts which reply sits where. Three more exchanges: no page errors, the window still there.
const db = `${process.env.MIMIR_HOME || '/tmp/flow-home'}/mimir.db`
const seed = `WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 250),
  c AS (SELECT chat_id, bot_id FROM messages WHERE content LIKE '%hello Ada%' LIMIT 1)`
execFileSync('sqlite3', [db, `${seed} INSERT INTO tool_calls(run_id, chat_id, tool, summary, decision, ok, result, ts)
  SELECT 900000 + i, c.chat_id, CASE i % 4 WHEN 0 THEN 'memory_store' ELSE 'memory_search' END, 'Remember "Seed ' || i || '": x', 'allow', 1, 'stored #1', 0 FROM n, c WHERE i % 2 = 0;
${seed} INSERT INTO messages(chat_id, bot_id, role, content, ts, run_id)
  SELECT c.chat_id, c.bot_id, CASE i % 2 WHEN 1 THEN 'user' ELSE 'assistant' END, 'seed ' || i, strftime('%s', 'now'), CASE i % 2 WHEN 0 THEN 900000 + i END FROM n, c;`])
const before = errs.length
await bot('Ada').click(); await tab('Chat').click(); await p.waitForTimeout(1000)
const shown = () => p.locator('.main').innerText({ timeout: 2000 }).catch(() => '')
for (const k of [1, 2, 3]) { await say(`long chat ${k}`).catch(() => {}); await until(async () => (await shown()).includes(`OK: long chat ${k}`), 8000); await p.waitForTimeout(4500) }
check('a 250-message chat takes 3 more exchanges', (await shown()).includes('OK: long chat 3') && errs.length === before, errs.slice(before).join(' | '))

await p.screenshot({ path: '/tmp/ss/ui-flows-end.png' })
console.log(errs.length ? errs.join('\n') : 'no page errors')
console.log(failed ? `${failed} FAILED` : 'ALL PASS'); await b.close()
