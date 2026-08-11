import { describe, expect, it, vi } from 'vitest'
import ExplorerTree from '@/components/ExplorerTree.vue'
import { mountWithPlugins } from './mount'
import type { ExplorerNode } from '@/stores/explorer'

function node(overrides: Partial<ExplorerNode> = {}): ExplorerNode {
  return {
    key: 'db',
    label: 'Sales',
    kind: 'database',
    icon: 'mdi-database',
    children: [],
    loading: false,
    loaded: false,
    connectionId: 'c1',
    ...overrides,
  }
}

function mountTree(nodes: ExplorerNode[], openKeys = new Set<string>(), selectedKey?: string) {
  return mountWithPlugins(ExplorerTree, { props: { nodes, openKeys, selectedKey } })
}

describe('ExplorerTree', () => {
  it('draws one row for each node', () => {
    const wrapper = mountTree([node(), node({ key: 'db2', label: 'Other' })])
    expect(wrapper.findAll('[data-test="tree-row"]')).toHaveLength(2)
  })

  it('offers a chevron only on a node that can hold children', () => {
    const wrapper = mountTree([
      node(),
      node({ key: 'col', kind: 'column', label: 'id', children: undefined }),
    ])
    expect(wrapper.findAll('[data-test="tree-chevron"]')).toHaveLength(1)
  })

  it('reports whether a branch is open', () => {
    const wrapper = mountTree([node()], new Set(['db']))
    const row = wrapper.find('[role="treeitem"]')
    expect(row.attributes('aria-expanded')).toBe('true')
  })

  it('reports a closed branch and a leaf', () => {
    const wrapper = mountTree([node(), node({ key: 'col', kind: 'column', children: undefined })])
    const rows = wrapper.findAll('[role="treeitem"]')
    expect(rows[0]?.attributes('aria-expanded')).toBe('false')
    expect(rows[1]?.attributes('aria-expanded')).toBeUndefined()
  })

  it('asks the parent to open a node that was clicked', async () => {
    const wrapper = mountTree([node()])
    await wrapper.find('[data-test="tree-row"]').trigger('click')
    expect(wrapper.emitted('activate')?.[0]?.[0]).toMatchObject({ key: 'db' })
  })

  it('opens a node from the keyboard', async () => {
    const wrapper = mountTree([node()])
    await wrapper.trigger('keydown', { key: 'Enter' })
    await wrapper.trigger('keydown', { key: ' ' })
    expect(wrapper.emitted('activate')).toHaveLength(2)
  })

  it('reports a request for the menu of a node', async () => {
    const wrapper = mountTree([node()])
    await wrapper
      .find('[data-test="tree-row"]')
      .trigger('contextmenu', { clientX: 40, clientY: 90 })
    expect(wrapper.emitted('context')?.[0]?.[0]).toMatchObject({
      node: { key: 'db' },
      x: 40,
      y: 90,
    })
  })

  it('marks the selected node', () => {
    const wrapper = mountTree([node()], new Set(), 'db')
    expect(wrapper.find('[data-test="tree-row"]').classes()).toContain('selected')
  })

  it('shows the progress of a branch that reads', () => {
    const wrapper = mountTree([node({ loading: true })])
    expect(wrapper.find('[data-test="tree-loading"]').exists()).toBe(true)
  })

  it('draws the children of an open branch and passes their events up', async () => {
    const child = node({ key: 'schema', label: 'dbo', kind: 'schema' })
    const wrapper = mountTree([node({ children: [child] })], new Set(['db']))
    const rows = wrapper.findAll('[data-test="tree-row"]')
    expect(rows).toHaveLength(2)

    await rows[1]!.trigger('click')
    expect(wrapper.emitted('activate')?.[0]?.[0]).toMatchObject({ key: 'schema' })

    await rows[1]!.trigger('contextmenu')
    expect(wrapper.emitted('context')?.[0]?.[0]).toMatchObject({ node: { key: 'schema' } })
  })

  it('says so when a branch that was read holds nothing', () => {
    const wrapper = mountTree([node({ loaded: true, children: [] })], new Set(['db']))
    expect(wrapper.text()).toContain('Nothing here')
  })

  it('shows the type of a column beside its name', () => {
    const wrapper = mountTree([
      node({ key: 'col', kind: 'column', label: 'id', hint: 'int not null', children: undefined }),
    ])
    expect(wrapper.find('.node-hint').text()).toBe('int not null')
  })

  it('shows the whole name of a long row, which the panel scrolls to', () => {
    const wrapper = mountTree([node({ label: 'a_very_long_table_name_indeed' })])
    const label = wrapper.find('.node-label')
    expect(label.text()).toBe('a_very_long_table_name_indeed')
    // The name is not cut short, so it needs no second copy under the pointer.
    expect(label.attributes('title')).toBeUndefined()
  })
})

describe('ExplorerTree as a tree a reader can follow', () => {
  it('names itself a tree and its rows the items of one', () => {
    const wrapper = mountTree([node()])

    expect(wrapper.attributes('role')).toBe('tree')
    expect(wrapper.attributes('aria-label')).toBe('Database objects')
    expect(wrapper.find('[data-test="tree-row"]').attributes('role')).toBe('treeitem')
  })

  it('gives each row its level and its place among its own kind', () => {
    const child = node({ key: 'schema', label: 'dbo', kind: 'schema' })
    const wrapper = mountTree(
      [node({ children: [child] }), node({ key: 'db2', label: 'Other' })],
      new Set(['db']),
    )
    const rows = wrapper.findAll('[data-test="tree-row"]')

    expect(rows[0]!.attributes('aria-level')).toBe('1')
    expect(rows[0]!.attributes('aria-posinset')).toBe('1')
    expect(rows[0]!.attributes('aria-setsize')).toBe('2')
    expect(rows[1]!.attributes('aria-level')).toBe('2')
    expect(rows[1]!.attributes('aria-posinset')).toBe('1')
    expect(rows[1]!.attributes('aria-setsize')).toBe('1')
  })

  it('says which row the user chose', () => {
    const wrapper = mountTree([node(), node({ key: 'db2' })], new Set(), 'db2')
    const rows = wrapper.findAll('[data-test="tree-row"]')

    expect(rows[0]!.attributes('aria-selected')).toBe('false')
    expect(rows[1]!.attributes('aria-selected')).toBe('true')
  })

  it('holds one tab stop, whatever the number of its rows', () => {
    const wrapper = mountTree([node(), node({ key: 'db2' }), node({ key: 'db3' })])
    const stops = wrapper
      .findAll('[data-test="tree-row"]')
      .filter((row) => row.attributes('tabindex') === '0')

    expect(stops).toHaveLength(1)
  })

  it('puts the tab stop on the row the user chose', () => {
    const wrapper = mountTree([node(), node({ key: 'db2' })], new Set(), 'db2')
    const rows = wrapper.findAll('[data-test="tree-row"]')

    expect(rows[0]!.attributes('tabindex')).toBe('-1')
    expect(rows[1]!.attributes('tabindex')).toBe('0')
  })

  it('falls back on the first row when the chosen row is not in the tree', () => {
    const wrapper = mountTree([node()], new Set(), 'gone')

    expect(wrapper.find('[data-test="tree-row"]').attributes('tabindex')).toBe('0')
  })

  it('moves the tab stop down and up the rows the user can see', async () => {
    const wrapper = mountTree([node(), node({ key: 'db2' }), node({ key: 'db3' })])

    await wrapper.trigger('keydown', { key: 'ArrowDown' })
    expect(tabStop(wrapper)).toBe(1)

    await wrapper.trigger('keydown', { key: 'ArrowDown' })
    expect(tabStop(wrapper)).toBe(2)

    await wrapper.trigger('keydown', { key: 'ArrowUp' })
    expect(tabStop(wrapper)).toBe(1)
  })

  it('keeps the place of the scroll across when it moves the focus', async () => {
    const wrapper = mountTree([node(), node({ key: 'db2' })])
    const row = wrapper.findAll('[data-test="tree-row"]')[1]!.element as HTMLElement
    const focus = vi.spyOn(row, 'focus')
    const scroll = vi.fn()
    row.scrollIntoView = scroll

    await wrapper.trigger('keydown', { key: 'ArrowDown' })
    await wrapper.vm.$nextTick()

    expect(focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest', inline: 'nearest' })
    wrapper.unmount()
  })

  it('stays where it is at the top and at the bottom', async () => {
    const wrapper = mountTree([node(), node({ key: 'db2' })])

    await wrapper.trigger('keydown', { key: 'ArrowUp' })
    expect(tabStop(wrapper)).toBe(0)

    await wrapper.trigger('keydown', { key: 'End' })
    await wrapper.trigger('keydown', { key: 'ArrowDown' })
    expect(tabStop(wrapper)).toBe(1)
  })

  it('reaches the first row and the last row', async () => {
    const wrapper = mountTree([node(), node({ key: 'db2' }), node({ key: 'db3' })])

    await wrapper.trigger('keydown', { key: 'End' })
    expect(tabStop(wrapper)).toBe(2)

    await wrapper.trigger('keydown', { key: 'Home' })
    expect(tabStop(wrapper)).toBe(0)
  })

  it('opens a shut branch with the right key', async () => {
    const wrapper = mountTree([node()])

    await wrapper.trigger('keydown', { key: 'ArrowRight' })

    expect(wrapper.emitted('expand')?.[0]?.[0]).toMatchObject({ key: 'db' })
  })

  it('moves into an open branch with the right key', async () => {
    const child = node({ key: 'schema', kind: 'schema' })
    const wrapper = mountTree([node({ children: [child] })], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'ArrowRight' })

    expect(wrapper.emitted('expand')).toBeUndefined()
    expect(tabStop(wrapper)).toBe(1)
  })

  it('leaves a leaf alone when the right key arrives', async () => {
    const wrapper = mountTree([node({ key: 'col', kind: 'column', children: undefined })])

    await wrapper.trigger('keydown', { key: 'ArrowRight' })

    expect(wrapper.emitted('expand')).toBeUndefined()
  })

  it('shuts an open branch with the left key', async () => {
    const wrapper = mountTree([node()], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'ArrowLeft' })

    expect(wrapper.emitted('collapse')?.[0]?.[0]).toMatchObject({ key: 'db' })
  })

  it('moves out to the row that holds a child with the left key', async () => {
    const child = node({ key: 'schema', kind: 'schema' })
    const wrapper = mountTree([node({ children: [child] })], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'End' })
    expect(tabStop(wrapper)).toBe(1)

    await wrapper.trigger('keydown', { key: 'ArrowLeft' })

    expect(wrapper.emitted('collapse')).toBeUndefined()
    expect(tabStop(wrapper)).toBe(0)
  })

  it('stays where it is when the left key arrives on a row of the first level', async () => {
    const wrapper = mountTree([node({ key: 'col', kind: 'column', children: undefined })])

    await wrapper.trigger('keydown', { key: 'ArrowLeft' })

    expect(tabStop(wrapper)).toBe(0)
  })

  it('reaches a row by the first letter of its name', async () => {
    const wrapper = mountTree([
      node({ key: 'a', label: 'Accounts' }),
      node({ key: 'b', label: 'Billing' }),
      node({ key: 'c', label: 'Customers' }),
    ])

    await wrapper.trigger('keydown', { key: 'c' })

    expect(tabStop(wrapper)).toBe(2)
  })

  it('starts a new word once the letters of the last one have run out', async () => {
    vi.useFakeTimers()
    const wrapper = mountTree([
      node({ key: 'a', label: 'Accounts' }),
      node({ key: 'b', label: 'Billing' }),
      node({ key: 'c', label: 'Customers' }),
    ])

    await wrapper.trigger('keydown', { key: 'c' })
    expect(tabStop(wrapper)).toBe(2)

    // Close upon each other, `c` and `b` would build one word that no name
    // begins with. After the pause they are two words of one letter each.
    vi.advanceTimersByTime(1000)
    await wrapper.trigger('keydown', { key: 'b' })

    expect(tabStop(wrapper)).toBe(1)
    vi.useRealTimers()
  })

  it('leaves the tree alone for a word no name begins with', async () => {
    const wrapper = mountTree([
      node({ key: 'a', label: 'Accounts' }),
      node({ key: 'c', label: 'Customers' }),
    ])

    await wrapper.trigger('keydown', { key: 'c' })
    expect(tabStop(wrapper)).toBe(1)

    await wrapper.trigger('keydown', { key: 'x' })

    expect(tabStop(wrapper)).toBe(1)
  })

  it('reaches the next row of the same letter when the letter comes twice', async () => {
    const wrapper = mountTree([
      node({ key: 'a', label: 'Sales' }),
      node({ key: 'b', label: 'Stock' }),
    ])

    await wrapper.trigger('keydown', { key: 's' })
    expect(tabStop(wrapper)).toBe(1)

    await wrapper.trigger('keydown', { key: 's' })
    expect(tabStop(wrapper)).toBe(0)
  })

  it('holds the letters together while they arrive close upon each other', async () => {
    const wrapper = mountTree([
      node({ key: 'a', label: 'Sales' }),
      node({ key: 'b', label: 'Stock' }),
    ])

    await wrapper.trigger('keydown', { key: 's' })
    await wrapper.trigger('keydown', { key: 't' })

    expect(tabStop(wrapper)).toBe(1)
  })

  it('leaves the tree alone for letters no name begins with', async () => {
    const wrapper = mountTree([node({ label: 'Sales' }), node({ key: 'db2', label: 'Stock' })])

    await wrapper.trigger('keydown', { key: 'z' })

    expect(tabStop(wrapper)).toBe(0)
  })

  it('leaves a key of the application to the application', async () => {
    const wrapper = mountTree([node({ label: 'Sales' }), node({ key: 'db2', label: 'Stock' })])

    await wrapper.trigger('keydown', { key: 's', ctrlKey: true })

    expect(tabStop(wrapper)).toBe(0)
  })

  it('opens the menu of a row with the keys of the host', async () => {
    const wrapper = mountTree([node()])

    await wrapper.trigger('keydown', { key: 'F10', shiftKey: true })
    expect(wrapper.emitted('context')?.[0]?.[0]).toMatchObject({ node: { key: 'db' } })

    await wrapper.trigger('keydown', { key: 'ContextMenu' })
    expect(wrapper.emitted('context')).toHaveLength(2)
  })

  it('leaves the F10 key alone without the shift key', async () => {
    const wrapper = mountTree([node()])

    await wrapper.trigger('keydown', { key: 'F10' })

    expect(wrapper.emitted('context')).toBeUndefined()
  })

  it('answers no key while it holds no rows', async () => {
    const wrapper = mountTree([])

    await wrapper.trigger('keydown', { key: 'ArrowDown' })

    expect(wrapper.emitted('expand')).toBeUndefined()
  })

  it('keeps the note of an empty branch out of the reach of the keys', async () => {
    const wrapper = mountTree([node({ loaded: true, children: [] })], new Set(['db']))
    expect(wrapper.text()).toContain('Nothing here')

    await wrapper.trigger('keydown', { key: 'ArrowDown' })

    expect(tabStop(wrapper)).toBe(0)
  })
})

/** The place of the row that carries the one tab stop of the tree. */
function tabStop(wrapper: ReturnType<typeof mountTree>): number {
  return wrapper
    .findAll('[data-test="tree-row"]')
    .findIndex((row) => row.attributes('tabindex') === '0')
}

describe('ExplorerTree with a branch that holds no list', () => {
  it('says so when a branch that was read holds no list at all', () => {
    const wrapper = mountTree([node({ loaded: true, children: undefined })], new Set(['db']))
    expect(wrapper.text()).toContain('Nothing here')
  })
})
