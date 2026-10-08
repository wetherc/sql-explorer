import { describe, expect, it, vi } from 'vitest'
import { nextTick, reactive } from 'vue'
import ExplorerTree from '@/components/ExplorerTree.vue'
import { mountWithPlugins } from './mount'
import type { ExplorerNode } from '@/stores/explorer'

function node(overrides: Partial<ExplorerNode> = {}): ExplorerNode {
  return {
    key: 'db',
    label: 'Sales',
    nodeType: 'database',
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
      node({ key: 'col', nodeType: 'column', label: 'id', children: undefined }),
    ])
    expect(wrapper.findAll('[data-test="tree-chevron"]')).toHaveLength(1)
  })

  it('reports whether a branch is open', () => {
    const wrapper = mountTree([node()], new Set(['db']))
    const row = wrapper.find('[role="treeitem"]')
    expect(row.attributes('aria-expanded')).toBe('true')
  })

  it('reports a closed branch and a leaf', () => {
    const wrapper = mountTree([
      node(),
      node({ key: 'col', nodeType: 'column', children: undefined }),
    ])
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
    const child = node({ key: 'schema', label: 'dbo', nodeType: 'schema' })
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
      node({
        key: 'col',
        nodeType: 'column',
        label: 'id',
        hint: 'int not null',
        children: undefined,
      }),
    ])
    expect(wrapper.find('.node-hint').text()).toBe('int not null')
  })

  it('dims an object that the engine does not run', () => {
    const wrapper = mountTree([
      node({ key: 'on', nodeType: 'trigger', label: 'audit', children: undefined }),
      node({ key: 'off', nodeType: 'trigger', label: 'old', children: undefined, dimmed: true }),
    ])
    const rows = wrapper.findAll('[data-test="tree-row"]')
    expect(rows[0]?.classes()).not.toContain('dimmed')
    expect(rows[1]?.classes()).toContain('dimmed')
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
  it('gives the note of an empty branch a level in the tree', () => {
    const wrapper = mountTree([node({ loaded: true, children: [] })], new Set(['db']))
    const note = wrapper.find('[data-test="tree-empty"]')
    expect(note.attributes('role')).toBe('treeitem')
    expect(note.attributes('aria-level')).toBe('2')
    expect(note.attributes('aria-disabled')).toBe('true')
  })

  it('shows a failed read of an open branch with a Retry button', async () => {
    const failed = { ...node(), error: 'The server went away.' }
    const wrapper = mountTree([failed], new Set(['db']))
    const note = wrapper.find('[data-test="tree-error"]')
    expect(note.text()).toContain("Couldn't load: The server went away.")
    expect(wrapper.text()).not.toContain('Nothing here')
    await wrapper.find('[data-test="tree-retry"]').trigger('click')
    expect(wrapper.emitted('retry')?.[0]?.[0]).toMatchObject({ key: 'db' })
    expect(wrapper.emitted('activate')).toBeUndefined()
    expect(wrapper.find('[data-test="tree-blocking"]').exists()).toBe(false)
  })

  it('offers the blocking sessions for a read that waited for a lock', async () => {
    const failed = { ...node(), error: 'Another session has locked this object.', lockWait: true }
    const wrapper = mountTree([failed], new Set(['db']))
    await wrapper.find('[data-test="tree-blocking"]').trigger('click')
    expect(wrapper.emitted('blocking')?.[0]?.[0]).toMatchObject({ key: 'db' })
    expect(wrapper.emitted('activate')).toBeUndefined()
  })

  it('tells a reader when a branch reads', () => {
    const busy = mountWithPlugins(ExplorerTree, {
      props: { nodes: [node({ loading: true })], openKeys: new Set(), busy: true },
    })
    expect(busy.attributes('aria-busy')).toBe('true')
    expect(mountTree([node()]).attributes('aria-busy')).toBe('false')
  })

  it('builds no row again when a branch starts or ends a read', async () => {
    const branch = reactive(node({ loaded: true, children: [] }))
    const wrapper = mountTree([branch], new Set(['db']))
    const rows = (wrapper.vm as unknown as { rows: unknown[] }).rows
    branch.loading = true
    await nextTick()
    expect((wrapper.vm as unknown as { rows: unknown[] }).rows).toBe(rows)
  })

  it('takes the tab stop itself while its row is out of view, and passes the focus on', async () => {
    const many = Array.from({ length: 400 }, (_item, index) =>
      node({ key: `db${index}`, label: `Node ${index}` }),
    )
    const wrapper = mountTree(many, new Set(), 'db0')
    expect(wrapper.attributes('tabindex')).toBe('-1')

    Object.defineProperty(wrapper.element, 'scrollTop', { value: 24 * 200, writable: true })
    await wrapper.trigger('scroll')
    expect(wrapper.attributes('tabindex')).toBe('0')

    ;(wrapper.element as HTMLElement).focus()
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(document.activeElement?.textContent).toContain('Node 0')
  })

  it('names itself a tree and its rows the items of one', () => {
    const wrapper = mountTree([node()])

    expect(wrapper.attributes('role')).toBe('tree')
    expect(wrapper.attributes('aria-label')).toBe('Database objects')
    expect(wrapper.find('[data-test="tree-row"]').attributes('role')).toBe('treeitem')
  })

  it('gives each row its level and its place among its siblings', () => {
    const child = node({ key: 'schema', label: 'dbo', nodeType: 'schema' })
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

  it('draws the rows around the visible part alone', async () => {
    const many = Array.from({ length: 400 }, (_item, index) =>
      node({ key: `db${index}`, label: `Node ${index}` }),
    )
    const wrapper = mountTree(many)

    // The window holds far fewer rows than the tree, and the space above
    // and below carries the rest.
    const drawn = wrapper.findAll('[data-test="tree-row"]').length
    expect(drawn).toBeGreaterThan(0)
    expect(drawn).toBeLessThan(many.length)

    const first = () => wrapper.find('[data-test="tree-row"]').text()
    expect(first()).toContain('Node 0')

    // A scroll of the area moves the window.
    Object.defineProperty(wrapper.element, 'scrollTop', { value: 24 * 200, writable: true })
    await wrapper.trigger('scroll')
    expect(first()).not.toContain('Node 0')

    // The End key reaches the last row, which the window did not hold.
    await wrapper.trigger('keydown', { key: 'End' })
    await wrapper.vm.$nextTick()
    const rows = wrapper.findAll('[data-test="tree-row"]')
    expect(rows[rows.length - 1]!.text()).toContain('Node 399')
    expect(rows[rows.length - 1]!.attributes('tabindex')).toBe('0')

    // The Home key reaches the first row again.
    await wrapper.trigger('keydown', { key: 'Home' })
    await wrapper.vm.$nextTick()
    expect(first()).toContain('Node 0')
  })

  it('follows the height of its area, and works without a watcher of it', async () => {
    const many = Array.from({ length: 400 }, (_item, index) =>
      node({ key: `db${index}`, label: `Node ${index}` }),
    )
    const callbacks: Array<() => void> = []
    class ObserverStub {
      constructor(callback: () => void) {
        callbacks.push(callback)
      }
      observe(): void {}
      disconnect(): void {}
    }
    const held = globalThis.ResizeObserver
    globalThis.ResizeObserver = ObserverStub as unknown as typeof ResizeObserver
    try {
      const wrapper = mountTree(many)
      const tall = wrapper.findAll('[data-test="tree-row"]').length

      // A height of none says nothing, so the window stays as it was.
      Object.defineProperty(wrapper.element, 'clientHeight', { value: 0, configurable: true })
      callbacks.forEach((callback) => callback())
      await wrapper.vm.$nextTick()
      expect(wrapper.findAll('[data-test="tree-row"]').length).toBe(tall)

      // A short area draws fewer rows than a tall one.
      Object.defineProperty(wrapper.element, 'clientHeight', { value: 48, configurable: true })
      callbacks.forEach((callback) => callback())
      await wrapper.vm.$nextTick()
      expect(wrapper.findAll('[data-test="tree-row"]').length).toBeLessThan(tall)
      wrapper.unmount()
    } finally {
      globalThis.ResizeObserver = held
    }

    // A host without the watcher draws the rows all the same.
    // @ts-expect-error the test takes the watcher away from the host.
    delete globalThis.ResizeObserver
    try {
      const wrapper = mountTree(many)
      expect(wrapper.findAll('[data-test="tree-row"]').length).toBeGreaterThan(0)
      // The area is gone once the tree goes away, so a move of the focus
      // reaches no place of a scroll.
      wrapper.unmount()
      ;(wrapper.vm as unknown as { focusRow: (key: string) => void }).focusRow('db0')
    } finally {
      globalThis.ResizeObserver = held
    }
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
    const child = node({ key: 'schema', nodeType: 'schema' })
    const wrapper = mountTree([node({ children: [child] })], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'ArrowRight' })

    expect(wrapper.emitted('expand')).toBeUndefined()
    expect(tabStop(wrapper)).toBe(1)
  })

  it('leaves a leaf alone when the right key arrives', async () => {
    const wrapper = mountTree([node({ key: 'col', nodeType: 'column', children: undefined })])

    await wrapper.trigger('keydown', { key: 'ArrowRight' })

    expect(wrapper.emitted('expand')).toBeUndefined()
  })

  it('shuts an open branch with the left key', async () => {
    const wrapper = mountTree([node()], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'ArrowLeft' })

    expect(wrapper.emitted('collapse')?.[0]?.[0]).toMatchObject({ key: 'db' })
  })

  it('moves out to the row that holds a child with the left key', async () => {
    const child = node({ key: 'schema', nodeType: 'schema' })
    const wrapper = mountTree([node({ children: [child] })], new Set(['db']))

    await wrapper.trigger('keydown', { key: 'End' })
    expect(tabStop(wrapper)).toBe(1)

    await wrapper.trigger('keydown', { key: 'ArrowLeft' })

    expect(wrapper.emitted('collapse')).toBeUndefined()
    expect(tabStop(wrapper)).toBe(0)
  })

  it('stays where it is when the left key arrives on a row of the first level', async () => {
    const wrapper = mountTree([node({ key: 'col', nodeType: 'column', children: undefined })])

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

describe('ExplorerTree width', () => {
  it('takes the width of the widest row, also when the row is not drawn', () => {
    const nodes = Array.from({ length: 300 }, (_, index) =>
      node({ key: `n${index}`, label: index === 250 ? 'x'.repeat(100) : 'short' }),
    )
    nodes[1]!.nodeType = 'column'
    nodes[1]!.hint = 'y'.repeat(20)
    const wrapper = mountTree(nodes)
    expect(wrapper.findAll('[data-test="tree-row"]').length).toBeLessThan(300)
    const body = wrapper.find('[data-test="tree-body"]').element as HTMLElement
    // The indent, the offset of the label, 100 characters of 7 pixels, and
    // the end of the row.
    expect(body.style.getPropertyValue('--tree-width')).toBe(`${6 + 46 + 700 + 8}px`)
  })

  it('counts the hint of a row', async () => {
    const wrapper = mountTree([node({ nodeType: 'column', label: 'id', hint: 'y'.repeat(100) })])
    // Both fonts are known, so a new drawing reads them no more.
    await wrapper.setProps({ selectedKey: 'db' })
    const body = wrapper.find('[data-test="tree-body"]').element as HTMLElement
    expect(body.style.getPropertyValue('--tree-width')).toBe(`${6 + 46 + 14 + 8 + 10 + 700}px`)
  })
})

describe('ExplorerTree with a branch that holds no list', () => {
  it('says so when a branch that was read holds no list at all', () => {
    const wrapper = mountTree([node({ loaded: true, children: undefined })], new Set(['db']))
    expect(wrapper.text()).toContain('Nothing here')
  })
})
