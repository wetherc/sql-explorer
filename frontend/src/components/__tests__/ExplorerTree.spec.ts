import { describe, expect, it } from 'vitest'
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
    const row = wrapper.find('[data-test="tree-row"]')
    await row.trigger('keydown.enter')
    await row.trigger('keydown.space')
    expect(wrapper.emitted('activate')).toHaveLength(2)
  })

  it('reports a request for the menu of a node', async () => {
    const wrapper = mountTree([node()])
    await wrapper.find('[data-test="tree-row"]').trigger('contextmenu')
    expect(wrapper.emitted('context')?.[0]?.[0]).toMatchObject({ node: { key: 'db' } })
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
})
