import { beforeEach, describe, expect, it, vi } from 'vitest'
import { isReactive, nextTick } from 'vue'
import ResultsGrid from '@/components/ResultsGrid.vue'
import { mountWithPlugins } from './mount'
import { ResultTable } from '@/lib/results'
import type { CellValue, ColumnInfo, ResultSet } from '@/types/api'

const columns: ColumnInfo[] = [
  { name: 'id', typeName: 'int' },
  { name: 'name', typeName: 'text' },
]

const records: CellValue[][] = [
  [2, 'Grace'],
  [1, 'Ada'],
  [3, null],
]

/** The table one test draws. The grid reads the rows through the table. */
function result(
  overrides: { columns?: ColumnInfo[]; rows?: CellValue[][]; truncated?: boolean } = {},
): ResultTable {
  return ResultTable.fromRows(
    overrides.columns ?? columns,
    overrides.rows ?? records,
    overrides.truncated ?? false,
  )
}

/** Clicks one item of a menu that the library drew outside the wrapper. */
function click(selector: string): void {
  document.querySelector(selector)?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
}

describe('ResultsGrid', () => {
  beforeEach(() => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    })
  })

  it('draws one row for each record and names the columns with their types', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(3)
    const headers = wrapper.findAll('[data-test="grid-header"]')
    expect(headers[0]?.text()).toContain('id')
    expect(headers[0]?.text()).toContain('int')
  })

  it('marks a cell that holds no value', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const empty = wrapper.findAll('.null-cell')
    expect(empty).toHaveLength(1)
    expect(empty[0]?.text()).toBe('NULL')
  })

  it('counts the rows it shows', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('3 rows')
  })

  it('draws the rows that arrived while the set streams', async () => {
    const table = new ResultTable(columns)
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 0 } })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(0)

    // The table grows outside the reactivity of Vue, and the count of the
    // rows carries the growth to the grid.
    table.addSegment([], 2)
    await wrapper.setProps({ rows: 2 })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(2)
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('2 rows')
  })

  it('reports that the row limit stopped the read', () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ truncated: true }) },
    })
    expect(wrapper.find('[data-test="grid-truncated"]').text()).toContain('row limit')
  })

  it('takes the mark of the row limit from the pane over the table', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result(), truncated: false },
    })
    expect(wrapper.find('[data-test="grid-truncated"]').exists()).toBe(false)
    await wrapper.setProps({ truncated: true })
    expect(wrapper.find('[data-test="grid-truncated"]').exists()).toBe(true)
  })

  it('says so when a statement returned no rows', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result({ rows: [] }) } })
    expect(wrapper.find('[data-test="grid-empty"]').text()).toBe('This statement returned no rows.')
  })

  it('says so when the filter matches no row', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
      await wrapper.find('[data-test="grid-filter"] input').setValue('nothing here')
      await vi.runAllTimersAsync()
      await wrapper.vm.$nextTick()

      expect(wrapper.find('[data-test="grid-empty"]').text()).toBe('No row matches the filter.')
    } finally {
      vi.useRealTimers()
    }
  })

  it('sorts up, then down, then not at all', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const header = wrapper.findAll('[data-test="grid-header"]')[0]!
    const cell = wrapper.findAll('[data-test="grid-header-cell"]')[0]!

    await header.trigger('click')
    expect(cell.attributes('aria-sort')).toBe('ascending')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Ada')

    await header.trigger('click')
    expect(cell.attributes('aria-sort')).toBe('descending')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('3')

    await header.trigger('click')
    expect(cell.attributes('aria-sort')).toBe('none')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Grace')
  })

  it('does not sort a new result by the sort of the result before it', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-header"]')[0]!.trigger('click')
    const next = result({ rows: Array.from({ length: 500 }, (_, index) => [index, `n${index}`]) })
    const cell = vi.spyOn(next, 'cell')
    await wrapper.setProps({ result: next })
    // The window reads the rows it draws. A sort would read the column of
    // every row.
    expect(cell.mock.calls.filter(([row]) => row >= 200)).toHaveLength(0)
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('n0')
  })

  it('keeps a selection of every row outside the deep reactivity', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[0]!.trigger('keydown', {
      key: 'a',
      ctrlKey: true,
    })
    const state = (wrapper.vm as unknown as { $: { setupState: { selected: Set<number> } } }).$
      .setupState.selected
    expect(isReactive(state)).toBe(false)
    expect(state.size).toBe(3)
  })

  it('stops the drag of a grip when the grid goes away', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const remove = vi.spyOn(globalThis, 'removeEventListener')
    await wrapper.find('[data-test="grid-column-grip"]').trigger('pointerdown', { clientX: 10 })
    wrapper.unmount()
    expect(remove).toHaveBeenCalledWith('pointermove', expect.any(Function))
    remove.mockRestore()
  })

  it('puts the sort on a button, which a key can reach', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    expect(wrapper.findAll('[data-test="grid-header"]')[0]!.element.tagName).toBe('BUTTON')
  })

  it('moves the sort to another column', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const headers = wrapper.findAll('[data-test="grid-header"]')
    const cells = wrapper.findAll('[data-test="grid-header-cell"]')
    await headers[0]!.trigger('click')
    await headers[1]!.trigger('click')
    expect(cells[0]?.attributes('aria-sort')).toBe('none')
    expect(cells[1]?.attributes('aria-sort')).toBe('ascending')
  })

  it('keeps only the rows that match the filter after a short pause', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('ada')
    // The filter waits for a pause, so every row still shows.
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(3)
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('1 of 3 rows')
  })

  it('matches a row that arrived after the filter was set', async () => {
    const table = ResultTable.fromRows(columns, records)
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 3 } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('null')
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)

    // A row of nulls arrives while the filter stands, and the filter reads
    // the rows again when the count moves.
    table.addSegment([], 1)
    await wrapper.setProps({ rows: 4 })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(2)
  })

  it('sorts a row that arrived while the sort stands', async () => {
    const table = ResultTable.fromRows(columns, records)
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 3 } })
    await wrapper.findAll('[data-test="grid-header"]')[1]!.trigger('click')

    table.addSegment([], 1)
    await wrapper.setProps({ rows: 4 })
    const rows = wrapper.findAll('[data-test="grid-row"]')
    expect(rows).toHaveLength(4)
    expect(rows[0]?.text()).toContain('Ada')
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('4 rows')
  })

  it('matches and sorts a row that arrived while a filter and a sort stand', async () => {
    const table = ResultTable.fromRows(columns, records)
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 3 } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('null')
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.findAll('[data-test="grid-header"]')[0]!.trigger('click')
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)

    table.addSegment([], 1)
    await wrapper.setProps({ rows: 4 })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(2)
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('2 of 4 rows')
  })

  it('makes the sort of new rows wait after a slow sort', async () => {
    vi.useFakeTimers()
    // Each read of the clock moves it 100 ms on, so a sort takes 100 ms.
    let clock = 0
    const now = vi.spyOn(performance, 'now').mockImplementation(() => (clock += 100))
    try {
      const table = ResultTable.fromRows(columns, records)
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 3 } })
      await wrapper.findAll('[data-test="grid-header"]')[0]!.trigger('click')

      table.addSegment([], 1)
      await wrapper.setProps({ rows: 4 })
      // The sort waits, so the view holds the rows of the last sort.
      expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(3)

      // A second chunk during the wait adds no second timer.
      table.addSegment([], 1)
      await wrapper.setProps({ rows: 5 })
      await vi.runAllTimersAsync()
      await wrapper.vm.$nextTick()
      expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(5)

      // A timer that waits when the grid goes away is stopped.
      table.addSegment([], 1)
      await wrapper.setProps({ rows: 6 })
      wrapper.unmount()
      expect(vi.getTimerCount()).toBe(0)
    } finally {
      now.mockRestore()
      vi.useRealTimers()
    }
  })

  it('matches no row of a new result against the text of the old one', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('ada')
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)

    await wrapper.setProps({ result: result({ rows: [[7, 'Linus']] }) })
    const rows = wrapper.findAll('[data-test="grid-row"]')
    expect(rows).toHaveLength(1)
    expect(rows[0]?.text()).toContain('Linus')
  })

  it('opens the whole value of a cell', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('dblclick')
    expect(document.body.textContent).toContain('Grace')
  })

  it('copies the rows to the clipboard, with and without the column names', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-copy"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))

    click('[data-test="grid-copy-with-names"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(globalThis.navigator.clipboard.writeText).toHaveBeenCalledWith(
      'id\tname\n2\tGrace\n1\tAda\n3\tNULL',
    )
    expect(wrapper.emitted('copied')).toBeTruthy()

    click('[data-test="grid-copy-without-names"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(globalThis.navigator.clipboard.writeText).toHaveBeenCalledWith(
      '2\tGrace\n1\tAda\n3\tNULL',
    )
  })

  it('copies even when the host offers no clipboard', async () => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: undefined,
    })
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-copy"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    click('[data-test="grid-copy-with-names"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.emitted('copied')).toBeTruthy()
  })

  it('copies one cell and one row from the menu of the cells', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('contextmenu')
    await new Promise((resolve) => setTimeout(resolve, 0))

    click('[data-test="grid-menu-copy-cell"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(writeText).toHaveBeenCalledWith('Grace')

    click('[data-test="grid-menu-copy-row"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(writeText).toHaveBeenCalledWith('2\tGrace')

    click('[data-test="grid-menu-inspect"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.body.textContent).toContain('Grace')
  })

  it('holds no cell of the menu when the view holds no row', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('contextmenu')
    await new Promise((resolve) => setTimeout(resolve, 0))
    // The filter takes the rows away, so the place the menu holds is empty.
    await wrapper.find('[data-test="grid-filter"] input').setValue('nothing here')
    await new Promise((resolve) => setTimeout(resolve, 300))

    click('[data-test="grid-menu-copy-cell"]')
    click('[data-test="grid-menu-copy-row"]')
    click('[data-test="grid-menu-inspect"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.emitted('copied')).toBeUndefined()
  })

  it('changes the width of a column with the pointer and with the keys', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const grip = wrapper.findAll('[data-test="grid-column-grip"]')[0]!
    const header = () => wrapper.findAll('[data-test="grid-header-cell"]')[0]!

    // The drag of the grip gives the column the width it reaches.
    await grip.trigger('pointerdown', { clientX: 100 })
    globalThis.dispatchEvent(
      Object.assign(new Event('pointermove'), { clientX: 300 }) as PointerEvent,
    )
    await wrapper.vm.$nextTick()
    expect(header().attributes('style')).toContain('width: 200px')

    // The drag ends, so a later move of the pointer changes nothing.
    globalThis.dispatchEvent(new Event('pointerup'))
    globalThis.dispatchEvent(
      Object.assign(new Event('pointermove'), { clientX: 500 }) as PointerEvent,
    )
    await wrapper.vm.$nextTick()
    expect(header().attributes('style')).toContain('width: 200px')

    // The arrows change the width by one step, and the limit holds it.
    await grip.trigger('keydown', { key: 'ArrowRight' })
    expect(header().attributes('style')).toContain('width: 216px')
    await grip.trigger('keydown', { key: 'ArrowLeft' })
    expect(header().attributes('style')).toContain('width: 200px')
    for (let step = 0; step < 20; step += 1) {
      await grip.trigger('keydown', { key: 'ArrowLeft' })
    }
    expect(header().attributes('style')).toContain('width: 56px')

    // A key the grip does not hold changes nothing, and Enter gives the
    // column the width of its content again.
    await grip.trigger('keydown', { key: 'a' })
    expect(header().attributes('style')).toContain('width: 56px')
    await grip.trigger('keydown', { key: 'Enter' })
    expect(header().attributes('style')).toBeUndefined()

    // A new result starts with the width of the content of each column.
    await grip.trigger('keydown', { key: 'ArrowLeft' })
    expect(header().attributes('style')).toContain('width')
    await wrapper.setProps({ result: result() })
    expect(header().attributes('style')).toBeUndefined()
  })

  it('changes the width of a column that the double click reaches', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const grip = wrapper.findAll('[data-test="grid-column-grip"]')[0]!
    await grip.trigger('keydown', { key: 'ArrowRight' })
    await grip.trigger('dblclick')
    expect(
      wrapper.findAll('[data-test="grid-header-cell"]')[0]!.attributes('style'),
    ).toBeUndefined()
  })

  it('reports the scroll position so that only the visible rows are drawn', async () => {
    const many = ResultTable.fromRows(
      [{ name: 'n', typeName: 'int' }],
      Array.from({ length: 500 }, (_unused, index) => [index]),
    )
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: many } })
    expect(wrapper.findAll('[data-test="grid-row"]').length).toBeLessThan(500)

    const scroller = wrapper.find('.grid-scroll')
    Object.defineProperty(scroller.element, 'scrollTop', { configurable: true, value: 3000 })
    Object.defineProperty(scroller.element, 'clientHeight', { configurable: true, value: 300 })
    await scroller.trigger('scroll')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('89')
  })

  it('waits for the last of two quick changes of the filter', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const field = wrapper.find('[data-test="grid-filter"] input')
    await field.setValue('gr')
    await field.setValue('ada')
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)
    expect(wrapper.find('[data-test="grid-row"]').text()).toContain('Ada')
  })

  it('follows the height of its area, and works without a watcher of it', async () => {
    const many = ResultTable.fromRows(
      [{ name: 'n', typeName: 'int' }],
      Array.from({ length: 500 }, (_unused, index) => [index]),
    )
    // The library draws parts that watch their own size, so each call gets
    // an empty list of entries.
    const callbacks: Array<(entries: unknown[]) => void> = []
    class ObserverStub {
      constructor(callback: (entries: unknown[]) => void) {
        callbacks.push(callback)
      }
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    }
    const held = globalThis.ResizeObserver
    globalThis.ResizeObserver = ObserverStub as unknown as typeof ResizeObserver
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: many } })
      const tall = wrapper.findAll('[data-test="grid-row"]').length
      const scroller = wrapper.find('.grid-scroll').element

      // A height of none says nothing, so the window stays as it was.
      Object.defineProperty(scroller, 'clientHeight', { value: 0, configurable: true })
      callbacks.forEach((callback) => callback([]))
      await nextTick()
      expect(wrapper.findAll('[data-test="grid-row"]').length).toBe(tall)

      // A short area draws fewer rows than a tall one.
      Object.defineProperty(scroller, 'clientHeight', { value: 60, configurable: true })
      callbacks.forEach((callback) => callback([]))
      await nextTick()
      expect(wrapper.findAll('[data-test="grid-row"]').length).toBeLessThan(tall)
      wrapper.unmount()
    } finally {
      globalThis.ResizeObserver = held
    }

    // @ts-expect-error the test takes the watcher away from the host.
    delete globalThis.ResizeObserver
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
      expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(3)
      wrapper.unmount()
    } finally {
      globalThis.ResizeObserver = held
    }
  })

  it('keeps the height it knows when the host reports none', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const scroller = wrapper.find('.grid-scroll')
    Object.defineProperty(scroller.element, 'clientHeight', { configurable: true, value: 0 })
    await scroller.trigger('scroll')
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(3)
  })

  it('starts again at the top when a new result arrives', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-header"]')[0]!.trigger('click')
    await wrapper.find('[data-test="grid-filter"] input').setValue('ada')

    await wrapper.setProps({ result: result({ rows: [[9, 'New']] }) })
    expect(wrapper.findAll('[data-test="grid-header-cell"]')[0]?.attributes('aria-sort')).toBe(
      'none',
    )
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)
  })

  it('asks for an export in each form and gives the rows of the view', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))

    const items = [...document.querySelectorAll('[data-test="grid-export-item"]')]
    expect(items).toHaveLength(5)
    for (const item of items) {
      item.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    }

    const asked = wrapper.emitted('export') as Array<[string, ResultSet]>
    expect(asked.map((call) => call[0])).toEqual(['csv', 'json', 'markdown', 'insert', 'xlsx'])
    expect(asked[0]![1].rows).toHaveLength(3)
  })

  it('offers a whole export only for a result that the row limit stopped', async () => {
    const plain = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await plain.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.querySelector('[data-test="grid-export-all-csv"]')).toBeNull()

    const cut = mountWithPlugins(ResultsGrid, {
      props: { result: result({ truncated: true }) },
    })
    await cut.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    document
      .querySelector('[data-test="grid-export-all-csv"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    document
      .querySelector('[data-test="grid-export-all-json"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    document
      .querySelector('[data-test="grid-export-all-xlsx"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(cut.emitted('export-all')).toEqual([['csv'], ['json'], ['xlsx']])
  })

  it('names the export after the selection once rows are selected', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-row"]')[0]!.trigger('click')
    await wrapper.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))

    const items = [...document.querySelectorAll('[data-test="grid-export-item"]')]
    expect(items[0]?.textContent).toContain('the selected rows')
    items[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    const asked = wrapper.emitted('export') as Array<[string, ResultSet]>
    expect(asked[0]![1].rows).toEqual([[2, 'Grace']])
  })

  it('selects one row, adds a row, and reaches a run of rows', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const rows = () => wrapper.findAll('[data-test="grid-row"]')

    await rows()[0]!.trigger('click')
    expect(rows()[0]!.classes()).toContain('selected')
    expect(wrapper.find('[data-test="grid-count"]').text()).toContain('1 selected')

    // Control adds a row and takes it away again.
    await rows()[2]!.trigger('click', { ctrlKey: true })
    expect(rows()[2]!.classes()).toContain('selected')
    await rows()[2]!.trigger('click', { ctrlKey: true })
    expect(rows()[2]!.classes()).not.toContain('selected')

    // Shift reaches from the row of the last click that set the anchor.
    await rows()[0]!.trigger('click', { shiftKey: true })
    expect(rows().every((row) => row.classes().includes('selected'))).toBe(true)
  })

  it('holds the selection through a sort', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-row"]')[0]!.trigger('click')
    await wrapper.findAll('[data-test="grid-header"]')[0]!.trigger('click')

    // The row of Grace moves to the end of the sort and keeps its mark.
    const rows = wrapper.findAll('[data-test="grid-row"]')
    expect(rows[1]!.classes()).toContain('selected')
    expect(rows[0]!.classes()).not.toContain('selected')
  })

  it('clears the selection from the menu and when a new result arrives', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-row"]')[0]!.trigger('click')
    await wrapper.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    document
      .querySelector('[data-test="grid-clear-selection"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="grid-count"]').text()).not.toContain('selected')

    await wrapper.findAll('[data-test="grid-row"]')[0]!.trigger('click')
    await wrapper.setProps({ result: result({ rows: [[9, 'Nine']] }) })
    expect(wrapper.find('[data-test="grid-count"]').text()).not.toContain('selected')
  })

  it('copies the rows of the selection alone', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-row"]')[1]!.trigger('click')
    await wrapper.find('[data-test="grid-copy"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    click('[data-test="grid-copy-with-names"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(writeText).toHaveBeenCalledWith('id\tname\n1\tAda')
  })

  it('names a cell of a column it cannot find', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: ResultTable.fromRows([], [[1]]) },
    })
    await wrapper.findAll('[data-test="grid-cell"]')[0]!.trigger('dblclick')
    expect(document.body.textContent).toContain('1')
  })
})

describe('ResultsGrid inspection dialog', () => {
  it('copies the value it shows and then closes', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })

    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('dblclick')
    await new Promise((resolve) => setTimeout(resolve, 0))

    const buttons = [...document.querySelectorAll('.v-card-actions .v-btn')]
    buttons
      .find((button) => button.textContent?.includes('Copy'))
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await Promise.resolve()
    expect(writeText).toHaveBeenCalledWith('Grace')

    buttons
      .find((button) => button.textContent?.includes('Close'))
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await new Promise((resolve) => setTimeout(resolve, 0))
    wrapper.unmount()
  })

  it('sorts a row that is shorter than the header', async () => {
    const ragged = ResultTable.fromRows(
      [
        { name: 'a', typeName: 'int' },
        { name: 'b', typeName: 'int' },
      ],
      [[2], [1, 5]],
    )
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: ragged } })
    await wrapper.findAll('[data-test="grid-header"]')[1]!.trigger('click')
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(2)
  })
})

describe('ResultsGrid dialog state', () => {
  it('closes the inspection when the overlay reports it', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('dblclick')
    await new Promise((resolve) => setTimeout(resolve, 0))

    const dialog = wrapper.findComponent({ name: 'VDialog' })
    await dialog.vm.$emit('update:modelValue', false)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(dialog.props('modelValue')).toBe(false)
  })
})

describe('ResultsGrid as a grid a reader can follow', () => {
  /** The place of the cell that carries the one tab stop, as row and column. */
  function tabStop(wrapper: ReturnType<typeof mountWithPlugins>): [number, number] | null {
    const rows = wrapper.findAll('[data-test="grid-row"]')
    for (const [rowIndex, row] of rows.entries()) {
      const column = row
        .findAll('[data-test="grid-cell"]')
        .findIndex((cell) => cell.attributes('tabindex') === '0')
      if (column >= 0) {
        return [rowIndex, column]
      }
    }
    return null
  }

  /** Gives one cell a width it shows and a width its whole value would need. */
  function setCellWidth(cell: { element: Element }, client: number, scroll: number) {
    const text = cell.element.querySelector('.cell-text') as HTMLElement
    Object.defineProperty(text, 'clientWidth', { configurable: true, value: client })
    Object.defineProperty(text, 'scrollWidth', { configurable: true, value: scroll })
  }

  function grid(wrapper: ReturnType<typeof mountWithPlugins>) {
    return wrapper.find('[role="grid"]')
  }

  it('names itself a grid and gives the count of all of its rows', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const table = grid(wrapper)

    expect(table.attributes('aria-label')).toBe('The rows of the result')
    // The count holds the row of the headers as well as the three rows.
    expect(table.attributes('aria-rowcount')).toBe('4')
    expect(table.attributes('aria-colcount')).toBe('3')
  })

  it('gives each row and each cell its place in the whole result', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const rows = wrapper.findAll('[data-test="grid-row"]')

    expect(rows[0]!.attributes('aria-rowindex')).toBe('2')
    expect(rows[2]!.attributes('aria-rowindex')).toBe('4')
    const cells = rows[0]!.findAll('[data-test="grid-cell"]')
    expect(cells[0]!.attributes('aria-colindex')).toBe('2')
    expect(cells[1]!.attributes('aria-colindex')).toBe('3')
  })

  it('names the parts of a row and of a column for a reader', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    expect(wrapper.find('[data-test="grid-header-cell"]').attributes('role')).toBe('columnheader')
    expect(wrapper.find('[data-test="grid-cell"]').attributes('role')).toBe('gridcell')
    expect(wrapper.find('td.row-number').attributes('role')).toBe('rowheader')
  })

  it('holds one tab stop, whatever the number of its cells', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const stops = wrapper
      .findAll('[data-test="grid-cell"]')
      .filter((cell) => cell.attributes('tabindex') === '0')

    expect(stops).toHaveLength(1)
    expect(tabStop(wrapper)).toEqual([0, 0])
  })

  it('moves the tab stop between the cells with the arrow keys', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'ArrowRight' })
    expect(tabStop(wrapper)).toEqual([0, 1])

    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })
    expect(tabStop(wrapper)).toEqual([1, 1])

    await grid(wrapper).trigger('keydown', { key: 'ArrowLeft' })
    expect(tabStop(wrapper)).toEqual([1, 0])

    await grid(wrapper).trigger('keydown', { key: 'ArrowUp' })
    expect(tabStop(wrapper)).toEqual([0, 0])
  })

  it('stays inside the grid at each of its edges', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'ArrowUp' })
    await grid(wrapper).trigger('keydown', { key: 'ArrowLeft' })
    expect(tabStop(wrapper)).toEqual([0, 0])

    await grid(wrapper).trigger('keydown', { key: 'End', ctrlKey: true })
    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })
    await grid(wrapper).trigger('keydown', { key: 'ArrowRight' })
    expect(tabStop(wrapper)).toEqual([2, 1])
  })

  it('reaches the ends of a row and the ends of the grid', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'End' })
    expect(tabStop(wrapper)).toEqual([0, 1])

    await grid(wrapper).trigger('keydown', { key: 'Home' })
    expect(tabStop(wrapper)).toEqual([0, 0])

    await grid(wrapper).trigger('keydown', { key: 'End', ctrlKey: true })
    expect(tabStop(wrapper)).toEqual([2, 1])

    await grid(wrapper).trigger('keydown', { key: 'Home', ctrlKey: true })
    expect(tabStop(wrapper)).toEqual([0, 0])
  })

  it('moves by a page of rows', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'PageDown' })
    expect(tabStop(wrapper)).toEqual([2, 0])

    await grid(wrapper).trigger('keydown', { key: 'PageUp' })
    expect(tabStop(wrapper)).toEqual([0, 0])
  })

  /**
   * Gives a scroll area that holds a position, because the test environment
   * draws nothing and reports a height of zero for each element.
   */
  function measure(element: Element, height: number): { position: number } {
    const box = { position: 0 }
    Object.defineProperty(element, 'scrollTop', {
      configurable: true,
      get: () => box.position,
      set: (value: number) => {
        box.position = value
      },
    })
    Object.defineProperty(element, 'clientHeight', { configurable: true, value: height })
    return box
  }

  it('keeps the focus on a jump past the rows that it draws', async () => {
    const many = ResultTable.fromRows(
      [{ name: 'n', typeName: 'int' }],
      Array.from({ length: 500 }, (_unused, index) => [index]),
    )
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: many } })
    const scroller = wrapper.find('.grid-scroll')
    const area = measure(scroller.element, 300)
    await scroller.trigger('scroll')

    await grid(wrapper).trigger('keydown', { key: 'End', ctrlKey: true })
    await nextTick()

    expect(area.position).toBeGreaterThan(0)
    expect(document.activeElement?.getAttribute('data-test')).toBe('grid-cell')
    expect(document.activeElement?.textContent).toContain('499')

    // A step to a row that already stands in the visible part holds the area
    // where it is.
    const reached = area.position
    await grid(wrapper).trigger('keydown', { key: 'ArrowUp' })
    await nextTick()
    expect(area.position).toBe(reached)
    expect(document.activeElement?.textContent).toContain('498')
  })

  it('does nothing with a key that arrives after the grid went away', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const element = grid(wrapper).element
    wrapper.unmount()
    // The area and the cells are gone, so the move reaches no scroll and no
    // cell takes the focus.
    element.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    await nextTick()
    expect(document.activeElement?.getAttribute('data-test')).not.toBe('grid-cell')
  })

  it('holds the row clear of the header when it scrolls up to it', async () => {
    const many = ResultTable.fromRows(
      [{ name: 'n', typeName: 'int' }],
      Array.from({ length: 500 }, (_unused, index) => [index]),
    )
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: many } })
    const scroller = wrapper.find('.grid-scroll')
    const area = measure(scroller.element, 300)
    Object.defineProperty(wrapper.find('thead tr').element, 'clientHeight', {
      configurable: true,
      value: 40,
    })
    await scroller.trigger('scroll')

    await grid(wrapper).trigger('keydown', { key: 'End', ctrlKey: true })
    await grid(wrapper).trigger('keydown', { key: 'PageUp' })
    await nextTick()

    // Row 489 starts at 14670 pixels, and the header covers 40 pixels of the
    // top of the area, so the area stands 40 pixels above the row.
    expect(area.position).toBe(14630)
    expect(document.activeElement?.textContent).toContain('489')
  })

  it('opens the whole value of a cell with the enter key', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'ArrowRight' })
    await grid(wrapper).trigger('keydown', { key: 'Enter' })

    expect(document.body.textContent).toContain('Grace')
  })

  it('leaves the keys of the header to the controls of the header', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const sort = wrapper.findAll('[data-test="grid-header"]')[1]!
    const grip = wrapper.findAll('[data-test="grid-column-grip"]')[0]!

    await sort.trigger('keydown', { key: 'Enter' })
    await sort.trigger('keydown', { key: ' ' })
    expect(document.querySelector('.app-code-block')).toBeNull()
    expect(wrapper.findAll('[data-test="grid-row"]')[0]!.classes()).not.toContain('selected')

    ;(grip.element as HTMLElement).focus()
    await grip.trigger('keydown', { key: 'ArrowRight' })
    await grip.trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(grip.element)
  })

  it('takes a row with the space bar', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: ' ' })
    expect(wrapper.findAll('[data-test="grid-row"]')[0]!.classes()).toContain('selected')

    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })
    await grid(wrapper).trigger('keydown', { key: ' ' })
    const rows = wrapper.findAll('[data-test="grid-row"]')
    expect(rows[0]!.classes()).not.toContain('selected')
    expect(rows[1]!.classes()).toContain('selected')
  })

  it('adds a row to the rows already taken with control and the space bar', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: ' ' })
    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })
    await grid(wrapper).trigger('keydown', { key: ' ', ctrlKey: true })

    const rows = wrapper.findAll('[data-test="grid-row"]')
    expect(rows[0]!.classes()).toContain('selected')
    expect(rows[1]!.classes()).toContain('selected')

    await grid(wrapper).trigger('keydown', { key: ' ', ctrlKey: true })
    expect(wrapper.findAll('[data-test="grid-row"]')[1]!.classes()).not.toContain('selected')
  })

  it('leaves a key it does not use to the application', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    const event = await grid(wrapper).trigger('keydown', { key: 'a' })

    expect(tabStop(wrapper)).toEqual([0, 0])
    expect(event).toBeUndefined()
  })

  it('answers no key while it holds no rows', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result({ rows: [] }) } })

    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })

    expect(tabStop(wrapper)).toBeNull()
  })

  it('follows the focus that a pointer puts on a cell', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const cells = wrapper.findAll('[data-test="grid-row"]')[1]!.findAll('[data-test="grid-cell"]')

    await cells[1]!.trigger('focus')

    expect(tabStop(wrapper)).toEqual([1, 1])
  })

  it('keeps the empty space above and below the drawn rows out of the reading', () => {
    const rows = Array.from({ length: 400 }, (_, index) => [index, `Name ${index}`])
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result({ rows }) } })

    const hidden = wrapper.findAll('tbody tr[aria-hidden="true"]')
    expect(hidden.length).toBeGreaterThan(0)
  })

  it('returns the tab stop to the first cell when a new result arrives', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await grid(wrapper).trigger('keydown', { key: 'End', ctrlKey: true })
    expect(tabStop(wrapper)).toEqual([2, 1])

    await wrapper.setProps({ result: result({ rows: [[9, 'New']] }) })

    expect(tabStop(wrapper)).toEqual([0, 0])
  })

  it('says that it is busy while a new statement runs', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result(), busy: true } })

    expect(wrapper.find('[data-test="grid-busy"]').exists()).toBe(true)
    expect(grid(wrapper).attributes('aria-busy')).toBe('true')

    await wrapper.setProps({ busy: false })
    expect(wrapper.find('[data-test="grid-busy"]').exists()).toBe(false)
  })

  it('holds the whole value under the pointer only when the cell cuts it short', async () => {
    const long = 'x'.repeat(200)
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ rows: [[1, long]] }) },
    })
    const cells = wrapper.findAll('[data-test="grid-cell"]')

    // The width a cell shows is known to the browser alone, so the two sizes
    // stand for a cell that cuts its value short and one that does not.
    setCellWidth(cells[1]!, 420, 900)
    setCellWidth(cells[0]!, 420, 100)

    await cells[1]!.trigger('mouseenter')
    expect(cells[1]!.attributes('title')).toBe(long)

    await cells[0]!.trigger('mouseenter')
    expect(cells[0]!.attributes('title')).toBeUndefined()
  })

  it('takes the tooltip away again when the cell grows to hold its value', async () => {
    const long = 'x'.repeat(200)
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ rows: [[1, long]] }) },
    })
    const cell = wrapper.findAll('[data-test="grid-cell"]')[1]!

    setCellWidth(cell, 420, 900)
    await cell.trigger('mouseenter')
    expect(cell.attributes('title')).toBe(long)

    setCellWidth(cell, 900, 900)
    await cell.trigger('mouseenter')
    expect(cell.attributes('title')).toBeUndefined()
  })

  /** A result of more rows than one slice of the filter reads. */
  function largeResult(): ResultTable {
    const rows: CellValue[][] = Array.from({ length: 6000 }, (_, index) => [
      index,
      index === 5999 ? 'zulu' : 'other',
    ])
    return ResultTable.fromRows(columns, rows)
  }

  it('reads a large result in slices and says that the filter builds', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: largeResult() } })
      await wrapper.find('[data-test="grid-filter"] input').setValue('zulu')
      vi.advanceTimersByTime(250)
      await wrapper.vm.$nextTick()

      // The first slice read a part of the rows, so the build goes on and
      // the rows on screen still stand under no filter.
      expect(wrapper.find('[data-test="grid-filtering"]').text()).toContain('%')
      expect(wrapper.find('[data-test="grid-count"]').text()).not.toContain('of')

      await vi.runAllTimersAsync()
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(false)
      expect(wrapper.find('[data-test="grid-count"]').text()).toContain('1 of')
    } finally {
      vi.useRealTimers()
    }
  })

  it('drops a build that runs when the filter clears', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: largeResult() } })
      const field = wrapper.find('[data-test="grid-filter"] input')
      await field.setValue('zulu')
      vi.advanceTimersByTime(250)
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(true)

      await field.setValue('')
      await vi.runAllTimersAsync()
      await wrapper.vm.$nextTick()

      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(false)
      expect(wrapper.find('[data-test="grid-count"]').text()).not.toContain('of')
    } finally {
      vi.useRealTimers()
    }
  })

  it('keeps the text it read when the filter changes during a build', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: largeResult() } })
      const field = wrapper.find('[data-test="grid-filter"] input')
      await field.setValue('zulu')
      vi.advanceTimersByTime(250)
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(true)

      // The second filter arrives while the build runs. The build goes on
      // from the row it reached, and the new filter follows it.
      await field.setValue('5999')
      await vi.runAllTimersAsync()
      await wrapper.vm.$nextTick()

      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(false)
      expect(wrapper.find('[data-test="grid-count"]').text()).toContain('1 of')
    } finally {
      vi.useRealTimers()
    }
  })

  it('drops a build that runs when the grid closes', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = mountWithPlugins(ResultsGrid, { props: { result: largeResult() } })
      await wrapper.find('[data-test="grid-filter"] input').setValue('zulu')
      vi.advanceTimersByTime(250)
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="grid-filtering"]').exists()).toBe(true)

      wrapper.unmount()
      await vi.runAllTimersAsync()
    } finally {
      vi.useRealTimers()
    }
  })

  it('takes the filter away when another result arrives', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('ada')
    await new Promise((resolve) => setTimeout(resolve, 250))
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)

    await wrapper.setProps({
      result: result({
        rows: [
          [4, 'Ada'],
          [5, 'Alan'],
        ],
      }),
    })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(2)
    expect(wrapper.find('[data-test="grid-count"]').text()).not.toContain('of')
  })

  it('sorts the rows that arrived while the set streams', async () => {
    const table = ResultTable.fromRows(columns, records)
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: table, rows: 3 } })
    await wrapper.findAll('[data-test="grid-header"]')[1]!.trigger('click')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Ada')

    // The keys of the sort hold one value for each row, so a row that
    // arrives after the sort brings its own key.
    table.addSegment([], 1)
    await wrapper.setProps({ rows: 4 })
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(4)
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Ada')
  })

  it('holds the whole value under the focus that a key brings to a cell', async () => {
    const long = 'x'.repeat(200)
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ rows: [[1, long]] }) },
    })
    const cell = wrapper.findAll('[data-test="grid-cell"]')[1]!

    setCellWidth(cell, 420, 900)
    await cell.trigger('focus')

    expect(cell.attributes('title')).toBe(long)
    expect(tabStop(wrapper)).toEqual([0, 1])
  })
})

describe('ResultsGrid copy and menu from the keyboard', () => {
  beforeEach(() => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    })
  })

  function grid(wrapper: ReturnType<typeof mountWithPlugins>) {
    return wrapper.find('[role="grid"]')
  }

  /** Sends the copy of the browser to the grid, with a clipboard or with none. */
  function copy(wrapper: ReturnType<typeof mountWithPlugins>, withData = true) {
    const data = new Map<string, string>()
    const event = new Event('copy', { bubbles: true, cancelable: true }) as ClipboardEvent
    Object.defineProperty(event, 'clipboardData', {
      value: withData ? { setData: (kind: string, text: string) => data.set(kind, text) } : null,
    })
    wrapper.find('[data-test="grid-cell"]').element.dispatchEvent(event)
    return { event, text: data.get('text/plain') }
  }

  it('copies the cell of the tab stop, or the selected rows', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await grid(wrapper).trigger('keydown', { key: 'ArrowRight' })

    const cell = copy(wrapper)
    expect(cell.event.defaultPrevented).toBe(true)
    expect(cell.text).toBe('Grace')
    expect(wrapper.emitted('copied')).toEqual([['Grace']])

    await wrapper.findAll('[data-test="grid-row"]')[1]!.trigger('click')
    expect(copy(wrapper).text).toBe('1\tAda')
    wrapper.unmount()
  })

  it('uses the clipboard of the host when the event holds none', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const { event } = copy(wrapper, false)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(event.defaultPrevented).toBe(true)
    expect(globalThis.navigator.clipboard.writeText).toHaveBeenCalledWith('2')
    wrapper.unmount()
  })

  it('leaves text that the user marked to the browser', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const marked = vi
      .spyOn(globalThis, 'getSelection')
      .mockReturnValue({ toString: () => 'Gra' } as Selection)
    try {
      const { event, text } = copy(wrapper)
      expect(event.defaultPrevented).toBe(false)
      expect(text).toBeUndefined()
    } finally {
      marked.mockRestore()
    }
    wrapper.unmount()
  })

  it('copies nothing when the view holds no row', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('nothing here')
    await new Promise((resolve) => setTimeout(resolve, 250))
    const event = new Event('copy', { bubbles: true, cancelable: true })
    grid(wrapper).element.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
    expect(wrapper.emitted('copied')).toBeUndefined()
    wrapper.unmount()
  })

  it('takes every row with Ctrl or Cmd and A', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const count = () => wrapper.find('[data-test="grid-count"]').text()

    // A key that the grid does not know stays with the browser.
    const other = new KeyboardEvent('keydown', { key: 'x', bubbles: true, cancelable: true })
    grid(wrapper).element.dispatchEvent(other)
    expect(other.defaultPrevented).toBe(false)

    await grid(wrapper).trigger('keydown', { key: 'a' })
    expect(count()).not.toContain('selected')

    await grid(wrapper).trigger('keydown', { key: 'a', ctrlKey: true })
    expect(count()).toContain('3 selected')

    await wrapper.findAll('[data-test="grid-row"]')[0]!.trigger('click')
    await grid(wrapper).trigger('keydown', { key: 'A', metaKey: true })
    expect(count()).toContain('3 selected')
    wrapper.unmount()
  })

  it('opens the menu of the cell with the menu key and with Shift+F10', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const menuItem = () => document.querySelector('[data-test="grid-menu-copy-cell"]')

    await grid(wrapper).trigger('keydown', { key: 'F10' })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(menuItem()).toBeNull()

    await grid(wrapper).trigger('keydown', { key: 'ArrowDown' })
    await grid(wrapper).trigger('keydown', { key: 'F10', shiftKey: true })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(menuItem()).not.toBeNull()

    // A choice in the menu closes it, and the focus goes back to the cell.
    click('[data-test="grid-menu-copy-cell"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(writeText).toHaveBeenCalledWith('1')
    const cells = wrapper.findAll('[data-test="grid-cell"]')
    expect(document.activeElement).toBe(cells[2]!.element)

    await grid(wrapper).trigger('keydown', { key: 'ContextMenu' })
    await new Promise((resolve) => setTimeout(resolve, 0))
    click('[data-test="grid-menu-copy-row"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(writeText).toHaveBeenCalledWith('1\tAda')
    wrapper.unmount()
  })

  it('gives the focus back to the cell after the dialog of a value', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await grid(wrapper).trigger('keydown', { key: 'ContextMenu' })
    await new Promise((resolve) => setTimeout(resolve, 0))
    click('[data-test="grid-menu-inspect"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.body.textContent).toContain('Grace')

    const close = [...document.querySelectorAll('button')].find(
      (button) => button.textContent?.trim() === 'Close',
    )
    close?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await new Promise((resolve) => setTimeout(resolve, 0))
    await nextTick()
    expect(document.activeElement).toBe(wrapper.find('[data-test="grid-cell"]').element)
    wrapper.unmount()
  })

  it('leaves the focus alone when a menu of the pointer closes', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    // The menu of a key comes first, and the pointer then opens the menu again.
    await grid(wrapper).trigger('keydown', { key: 'ContextMenu' })
    await wrapper.findAll('[data-test="grid-cell"]')[3]!.trigger('contextmenu')
    await new Promise((resolve) => setTimeout(resolve, 0))
    const outside = document.createElement('button')
    document.body.append(outside)
    outside.focus()
    click('[data-test="grid-menu-copy-cell"]')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.activeElement).toBe(outside)
    outside.remove()
    wrapper.unmount()
  })

  it('opens no menu for a key that arrives after the grid went away', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const element = grid(wrapper).element
    wrapper.unmount()
    element.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true }))
    await nextTick()
    expect(document.querySelector('[data-test="grid-menu-copy-cell"]')).toBeNull()
  })
})
