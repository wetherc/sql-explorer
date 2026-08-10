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

    await header.trigger('click')
    expect(header.attributes('aria-sort')).toBe('ascending')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Ada')

    await header.trigger('click')
    expect(header.attributes('aria-sort')).toBe('descending')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('3')

    await header.trigger('click')
    expect(header.attributes('aria-sort')).toBe('none')
    expect(wrapper.findAll('[data-test="grid-row"]')[0]?.text()).toContain('Grace')
  })

  it('moves the sort to another column', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    const headers = wrapper.findAll('[data-test="grid-header"]')
    await headers[0]!.trigger('click')
    await headers[1]!.trigger('click')
    expect(headers[0]?.attributes('aria-sort')).toBe('none')
    expect(headers[1]?.attributes('aria-sort')).toBe('ascending')
  })

  it('keeps only the rows that match the filter', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-filter"] input').setValue('ada')
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)
    expect(wrapper.find('[data-test="grid-count"]').text()).toBe('1 of 3 rows')
  })

  it('opens the whole value of a cell', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('click')
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
    expect(wrapper.findAll('[data-test="grid-header"]')[0]?.attributes('aria-sort')).toBe('none')
    expect(wrapper.findAll('[data-test="grid-row"]')).toHaveLength(1)
  })

  it('asks for an export in each form', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, { props: { result: result() } })
    await wrapper.find('[data-test="grid-export"]').trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))
    const items = [...document.querySelectorAll('.v-list-item')]
    const csv = items.find((item) => item.textContent?.includes('CSV'))
    const json = items.find((item) => item.textContent?.includes('JSON'))
    csv?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    json?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(wrapper.emitted('export')).toEqual([['csv'], ['json']])
  })

  it('names a cell of a column it cannot find', async () => {
    const wrapper = mountWithPlugins(ResultsGrid, {
      props: { result: { columns: [], rows: [[1]], truncated: false } },
    })
    await wrapper.findAll('[data-test="grid-cell"]')[0]!.trigger('click')
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
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('click')
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
    await wrapper.findAll('[data-test="grid-cell"]')[1]!.trigger('click')
    await new Promise((resolve) => setTimeout(resolve, 0))

    const dialog = wrapper.findComponent({ name: 'VDialog' })
    await dialog.vm.$emit('update:modelValue', false)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(dialog.props('modelValue')).toBe(false)
  })
})
