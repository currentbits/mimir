// node stills.mjs <outdir> <frame,frame,...> [scale] [composition: Mimir or Guide]
import { bundle } from '@remotion/bundler'
import { renderStill, selectComposition, openBrowser } from '@remotion/renderer'
import path from 'node:path'
const [out, list, scale = '0.5', id = 'Mimir'] = process.argv.slice(2)
const root = process.cwd()
const serveUrl = await bundle({ entryPoint: path.join(root, 'src/index.ts'), webpackOverride: (c) => ({ ...c, resolve: { ...c.resolve, modules: [path.join(root, 'node_modules'), 'node_modules'] } }) })
const browser = await openBrowser('chrome')
const composition = await selectComposition({ serveUrl, id, puppeteerInstance: browser })
for (const f of list.split(',').map(Number)) {
  await renderStill({ composition, serveUrl, frame: f, output: `${out}/f${String(f).padStart(5, '0')}.png`, scale: Number(scale), puppeteerInstance: browser })
  console.log('ok', f)
}
await browser.close({ silent: true })
