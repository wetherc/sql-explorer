/**
 * Takes the screenshots of the documentation. It serves the interface with
 * the sample backend of `shots/browser`, opens each scene of `scenes.ts` in
 * Google Chrome, and writes `docs/screenshots/<name>.png` and a WebP copy.
 *
 *     pnpm screenshots            every scene
 *     pnpm screenshots grid plan  the named scenes
 *
 * It needs Google Chrome and `cwebp` (`brew install webp`).
 */
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chromium, type Page } from 'playwright-core'
import { createServer } from 'vite'
import { scenes } from './scenes.ts'

const frontend = fileURLToPath(new URL('..', import.meta.url))
const output = fileURLToPath(new URL('../../docs/screenshots/', import.meta.url))
const WIDTH = 1440
const HEIGHT = 880
/** Wide enough for the types beside the columns in the tree. */
const PANEL_WIDTH = 360

/** Stops the capture when the interface shows a notice. The sample backend
 *  throws for a command name with no handler in its `commands` table, and
 *  the interface shows that error as a notice. */
async function assertNoNotice(page: Page, scene: string): Promise<void> {
  const notices = await page.locator('[data-test="notice"]').allInnerTexts()
  if (notices.length > 0) {
    throw new Error(`The scene ${scene} shows a notice: ${notices.join(' | ')}`)
  }
}

const wanted = process.argv.slice(2)
const chosen = wanted.length === 0 ? scenes : scenes.filter((scene) => wanted.includes(scene.name))
if (chosen.length !== Math.max(wanted.length, chosen.length)) {
  throw new Error(`Unknown scene in: ${wanted.join(', ')}`)
}

const server = await createServer({
  root: frontend,
  configFile: `${frontend}/vite.config.ts`,
  logLevel: 'warn',
  server: { port: 1430, strictPort: false },
  // Vite reloads the page when it finds a dependency late, and the reload
  // breaks the import of the first scene. The scan of this entry finds the
  // Tauri mocks before the first page asks for them.
  optimizeDeps: { entries: ['index.html', 'shots/index.html'] },
})
await server.listen()
const url = `${server.resolvedUrls!.local[0]}shots/index.html`
const browser = await chromium.launch({ channel: 'chrome' })

try {
  for (const scene of chosen) {
    const context = await browser.newContext({
      viewport: { width: WIDTH, height: HEIGHT },
      deviceScaleFactor: 1,
      colorScheme: scene.colorScheme ?? 'dark',
      locale: 'en-US',
      timezoneId: 'Europe/Amsterdam',
    })
    // The layout store reads this record when the window opens.
    await context.addInitScript(
      (layout) => localStorage.setItem('sql-explorer.layout', JSON.stringify(layout)),
      { panel: scene.panel ?? 'explorer', panelOpen: true, panelWidth: PANEL_WIDTH },
    )
    const page = await context.newPage()
    const errors: string[] = []
    page.on('pageerror', (error) => errors.push(error.message))
    await page.goto(url)
    await page.getByText('Orders of the month').first().waitFor()
    await scene.act(page)
    // The pointer would leave a tooltip or a hover mark in the picture.
    await page.mouse.move(WIDTH - 200, 12)
    await page.waitForTimeout(400)
    if (errors.length > 0) {
      throw new Error(`The scene ${scene.name} raised: ${errors.join(' | ')}`)
    }
    await assertNoNotice(page, scene.name)
    const png = `${output}${scene.name}.png`
    await page.screenshot({ path: png, animations: 'disabled', caret: 'hide' })
    if (scene.webp) {
      execFileSync('cwebp', ['-quiet', '-q', '82', png, '-o', `${output}${scene.name}.webp`])
    }
    process.stdout.write(`${scene.name}\n`)
    await context.close()
  }
} finally {
  await browser.close()
  await server.close()
}
