import { beforeEach, describe, expect, it, vi } from 'vitest'
import ResultsGrid from '@/components/ResultsGrid.vue'
import { mountWithPlugins } from './mount'
import type { ResultSet } from '@/types/api'

function result(overrides: Partial<ResultSet> = {}): ResultSet {
  return {
    columns: [
      { name: 'id', typeName: 'int' },
      { name: 'name', typeName: 'text' },
    ],
    rows: [
      [2, 'Grace'],
      [1, 'Ada'],
      [3, null],
    ],
    truncated: false,
    ...overrides,
  }
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

  it('reports that the row limit stopped the read', () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ truncated: true }) },
    })
    expect(wrapper.find('[data-test="grid-truncated"]').text()).toContain('row limit')
  })

  it('says so when a statement returned no rows', () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result({ rows: [] }) } })
    expect(wrapper.text()).toContain('This statement returned no rows.')
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

  it('opens the whole value of a cell', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('dblclick')
    expect(document.body.textContent).toContain('Grace')
  })

  it('copies the rows to the clipboard', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-copy"]').trigger('click')
    await Promise.resolve()
    expect(globalThis.navigator.clipboard.writeText).toHaveBeenCalledWith(
      'id\tname\n2\tGrace\n1\tAda\n3\t',
    )
    expect(wrapper.emitted('copied')).toBeTruthy()
  })

  it('copies even when the host offers no clipboard', async () => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: undefined,
    })
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-copy"]').trigger('click')
    await Promise.resolve()
    expect(wrapper.emitted('copied')).toBeTruthy()
  })

  it('reports the scroll position so that only the visible rows are drawn', async () => {
    const many: ResultSet = {
      columns: [{ name: 'n', typeName: 'int' }],
      rows: Array.from({ length: 500 }, (_, index) => [index]),
      truncated: false,
    }
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: many } })
    expect(wrapper.findAll('[data-test="grid-row"]').length).toBeLessThan(500)

    const scroller = wrapper.find('.grid-scroll')
    Object.defineProperty(scroller.element, 'scrollTop', { configurable: true, value: 3000 })
    Object.defineProperty(scroller.element, 'clientHeight', { configurable: true, value: 300 })
    await scroller.trigger('scroll')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('89')
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
    expect(cut.emitted('export-all')).toEqual([['csv'], ['json']])
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
    expect(writeText).toHaveBeenCalledWith('id\tname\n1\tAda')
  })

  it('names a cell of a column it cannot find', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: { columns: [], rows: [[1]], truncated: false } },
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
    const ragged = {
      columns: [
        { name: 'a', typeName: 'int' },
        { name: 'b', typeName: 'int' },
      ],
      rows: [[2], [1, 5]],
      truncated: false,
    }
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

  it('opens the whole value of a cell with the enter key', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })

    await grid(wrapper).trigger('keydown', { key: 'ArrowRight' })
    await grid(wrapper).trigger('keydown', { key: 'Enter' })

    expect(document.body.textContent).toContain('Grace')
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

  it('holds the whole value of a cell under the pointer only when it is cut short', () => {
    const long = 'x'.repeat(200)
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: result({ rows: [[1, long]] }) },
    })
    const cells = wrapper.findAll('[data-test="grid-cell"]')

    expect(cells[0]!.attributes('title')).toBeUndefined()
    expect(cells[1]!.attributes('title')).toBe(long)
  })
})
