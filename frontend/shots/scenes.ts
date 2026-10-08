// The functions that Playwright runs in the page use the DOM.
/// <reference lib="dom" />
/// <reference lib="dom.iterable" />
/**
 * The scenes of the screenshots. Each scene starts from a fresh window with
 * the sample workspace, where Shop and Lake are open, and drives the
 * interface to the view of one picture.
 */
import type { Locator, Page } from 'playwright-core'

export interface Scene {
  /** The file name, without the extension, in `docs/screenshots`. */
  name: string
  /** True for a picture that the website shows, which needs a WebP copy. */
  webp?: boolean
  colorScheme?: 'dark' | 'light'
  /** The side panel that the window opens with. The default is the explorer. */
  panel?: 'connections' | 'explorer' | 'files' | 'history'
  act: (page: Page) => Promise<void>
}

const ORDERS_PATH = ['Shop (PostgreSQL)', 'shop', 'public', 'Tables', 'orders', 'Columns']

/** The visible elements with this hook. Each open tab keeps its view in the
 *  page, so the hooks of the other tabs are there but hidden. */
function byTest(page: Page, id: string): Locator {
  return page.locator(`[data-test="${id}"]`).filter({ visible: true })
}

/** The row of the tree whose label is exactly this text. */
function treeRow(page: Page, label: string): Locator {
  return byTest(page, 'tree-row')
    .filter({ has: page.getByText(label, { exact: true }) })
    .first()
}

/** Opens each branch of the path in turn, and waits for its children. */
async function openTree(page: Page, path: string[]): Promise<void> {
  for (const label of path) {
    const row = treeRow(page, label)
    if ((await row.getAttribute('aria-expanded')) !== 'true') {
      await row.locator('[data-test="tree-chevron"]').click()
    }
    await page.waitForFunction(
      (text) =>
        [...document.querySelectorAll('[data-test="tree-row"]')].some(
          (element) =>
            element.textContent?.includes(text) && element.getAttribute('aria-expanded') === 'true',
        ),
      label,
    )
  }
  await byTest(page, 'tree-loading').waitFor({ state: 'detached' })
}

async function openTab(page: Page, title: string): Promise<void> {
  await page.getByRole('tab', { name: title }).click()
}

/** Runs the statement of the active tab and waits for its rows. */
async function run(page: Page): Promise<void> {
  await byTest(page, 'run-button').click()
  await byTest(page, 'grid-row').first().waitFor()
}

/** The cell of one column of one visible row of the grid, from 0. */
function cell(page: Page, row: number, column: number): Locator {
  return byTest(page, 'grid-row').nth(row).locator('[data-test="grid-cell"]').nth(column)
}

export const scenes: Scene[] = [
  {
    name: 'overview',
    webp: true,
    act: async (page) => {
      await openTree(page, ORDERS_PATH)
      await run(page)
    },
  },
  {
    name: 'connections',
    webp: true,
    panel: 'connections',
    act: async (page) => {
      await byTest(page, 'connection-item')
        .filter({ hasText: 'Warehouse' })
        .locator('[data-test="connection-menu"]')
        .click()
      await byTest(page, 'edit-connection').click()
      await page.getByText('Edit Warehouse').waitFor()
    },
  },
  {
    name: 'explorer-menu',
    webp: true,
    act: async (page) => {
      await openTree(page, ORDERS_PATH)
      await run(page)
      await treeRow(page, 'orders').click({ button: 'right' })
      await page.getByText('Script as CREATE').waitFor()
    },
  },
  {
    name: 'properties',
    act: async (page) => {
      await openTree(page, ORDERS_PATH)
      await treeRow(page, 'orders').click({ button: 'right' })
      await page.getByText('Properties', { exact: true }).click()
      await byTest(page, 'property-index').first().waitFor()
    },
  },
  {
    name: 'completion',
    webp: true,
    act: async (page) => {
      await openTree(page, ORDERS_PATH)
      await page.locator('.monaco-editor .view-lines').click()
      await page.keyboard.press('ControlOrMeta+A')
      await page.keyboard.type('select o.order_id, o.total_amount\nfrom public.orders o\nwhere o.')
      await page.locator('.suggest-widget.visible').waitFor()
    },
  },
  {
    name: 'parameters',
    act: async (page) => {
      await openTab(page, 'Orders of one country')
      await byTest(page, 'parameters-button').click()
      await byTest(page, 'parameters-confirm').waitFor()
    },
  },
  {
    name: 'grid',
    webp: true,
    act: async (page) => {
      await run(page)
      await page.getByPlaceholder('Filter rows').fill('shipped')
      await byTest(page, 'grid-header').filter({ hasText: 'items' }).click()
      await page.waitForTimeout(300)
      // Cells in the first columns, so the grid does not scroll sideways.
      await cell(page, 1, 1).click()
      await cell(page, 3, 2).click({ modifiers: ['Shift'] })
      // The click on a header near the right edge scrolls the grid to show
      // the whole header, and the shift-click also selects the text of the
      // page. The picture shows neither.
      await page.evaluate(() => {
        for (const element of document.querySelectorAll('.results-grid *')) {
          element.scrollLeft = 0
        }
        window.getSelection()?.removeAllRanges()
      })
    },
  },
  {
    name: 'export',
    act: async (page) => {
      await run(page)
      await byTest(page, 'grid-export').click()
      await byTest(page, 'grid-export-item').first().waitFor()
    },
  },
  {
    name: 'plan',
    webp: true,
    act: async (page) => {
      await byTest(page, 'plan-button').click()
      const estimated = byTest(page, 'plan-estimated')
      if (await estimated.isVisible()) {
        await estimated.click()
      }
      await byTest(page, 'grid-row').first().waitFor()
    },
  },
  {
    name: 'athena',
    webp: true,
    act: async (page) => {
      await openTab(page, 'Sessions of each channel')
      await run(page)
    },
  },
  {
    name: 'history',
    panel: 'history',
    act: async (page) => {
      await byTest(page, 'history-entry').first().waitFor()
    },
  },
  {
    name: 'palette',
    act: async (page) => {
      await page.keyboard.press('ControlOrMeta+Shift+P')
      await byTest(page, 'palette-item').first().waitFor()
    },
  },
]
