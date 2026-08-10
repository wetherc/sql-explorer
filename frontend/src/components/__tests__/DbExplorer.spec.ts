import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const DbExplorer = (await import('@/components/DbExplorer.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useExplorerStore } = await import('@/stores/explorer')
const { useSettingsStore } = await import('@/stores/settings')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')
const { TableKind } = await import('@/types/api')

/** Opens the menu of a node and returns the item with the given name. */
async function openMenu(wrapper: ReturnType<typeof mountWithPlugins>, rowIndex: number) {
  await wrapper.findAll('[data-test="tree-row"]')[rowIndex]!.trigger('contextmenu')
  await settle()
}

function menuItem(test: string): HTMLElement | null {
  return document.querySelector(`[data-test="${test}"]`)
}

describe('DbExplorer', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    apiStub.listTables.mockResolvedValue([{ name: 'orders', kind: TableKind.Table }])
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])
  })

  async function mountExplorer() {
    const wrapper = mountWithPlugins(DbExplorer)
    const connections = useConnectionsStore()
    await connections.load()
    return wrapper
  }

  it('points at the connections when none is open', () => {
    const wrapper = mountWithPlugins(DbExplorer)
    expect(wrapper.text()).toContain('No open connection')
    expect(wrapper.find('[data-test="explorer-open-connections"]').exists()).toBe(true)
  })

  it('asks the layout to show the connections', async () => {
    const wrapper = mountWithPlugins(DbExplorer)
    await wrapper.find('[data-test="explorer-open-connections"]').trigger('click')
    expect(wrapper.emitted('open-connections')).toHaveLength(1)
  })

  it('opens and closes a branch', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="tree-row"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="tree-row"]')).toHaveLength(2)

    await wrapper.findAll('[data-test="tree-row"]')[0]!.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="tree-row"]')).toHaveLength(1)
  })

  it('reaches the columns of a table', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    for (let level = 0; level < 4; level += 1) {
      await wrapper.findAll('[data-test="tree-row"]')[level]!.trigger('click')
      await settle()
      await wrapper.vm.$nextTick()
    }

    expect(apiStub.listColumns).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(wrapper.text()).toContain('int not null')
  })

  it('reads every root again on request', async () => {
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="explorer-refresh"]').trigger('click')
    await settle()
    expect(apiStub.listDatabases).toHaveBeenCalled()
  })

  it('keeps only the nodes that match the filter', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="explorer-filter"] input').setValue('nothing')
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Nothing matches the filter')
  })

  it('builds the preview statement in the backend and runs it', async () => {
    apiStub.previewQuery.mockResolvedValue('SELECT TOP 1000 * FROM [Sales].[dbo].[orders];')
    apiStub.executeQuery.mockResolvedValue({
      results: [],
      messages: [],
      rowsAffected: null,
      elapsedMs: 1,
    })
    apiStub.addHistoryEntry.mockResolvedValue([])

    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    const database = root.children![0]!
    await explorer.expand(database)
    const schema = database.children![0]!
    await explorer.expand(schema)
    explorer.roots = [root]
    await wrapper.vm.$nextTick()

    const table = schema.children![0]!
    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu', { clientX: 5, clientY: 5 }),
      node: table,
    })
    await settle()

    menuItem('menu-preview')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.previewQuery).toHaveBeenCalledWith({
      connectionId: 'c1',
      database: 'Sales',
      schemaName: 'dbo',
      tableName: 'orders',
      limit: 1000,
    })
    expect(useTabsStore().tabs[0]?.query).toBe('SELECT TOP 1000 * FROM [Sales].[dbo].[orders];')
    expect(apiStub.executeQuery).toHaveBeenCalled()
  })

  it('opens a preview without running it when the setting says so', async () => {
    apiStub.previewQuery.mockResolvedValue('SELECT TOP 1000 * FROM [t];')
    const wrapper = await mountExplorer()
    useSettingsStore().update({ autoRunPreview: false })

    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        kind: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
        table: 'orders',
      },
    })
    await settle()
    menuItem('menu-preview')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.executeQuery).not.toHaveBeenCalled()
  })

  it('reports a failure to build the preview statement', async () => {
    apiStub.previewQuery.mockRejectedValue({ kind: 'notConnected', message: 'gone', detail: null })
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        kind: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
      },
    })
    await settle()
    menuItem('menu-preview')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('opens a tab on the connection of the node', async () => {
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    menuItem('menu-new-query')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(useTabsStore().tabs[0]?.connectionId).toBe('c1')
  })

  it('reads one branch again from its menu', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    menuItem('menu-refresh')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.listDatabases).toHaveBeenCalledTimes(2)
  })

  it('closes a connection from its menu', async () => {
    apiStub.disconnect.mockResolvedValue(undefined)
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    menuItem('menu-disconnect')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.disconnect).toHaveBeenCalledWith('c1')
    expect(useExplorerStore().roots).toHaveLength(0)
  })

  it('copies the quoted name of a relation', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    apiStub.quoteIdentifier.mockResolvedValue('[orders]')

    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        kind: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
        table: 'orders',
      },
    })
    await settle()

    menuItem('menu-copy-name')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(writeText).toHaveBeenCalledWith('[orders]')
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('copies even when the host offers no clipboard', async () => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: undefined,
    })
    apiStub.quoteIdentifier.mockResolvedValue('[orders]')

    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        kind: 'view',
        icon: 'mdi-table-eye',
        loading: false,
        loaded: false,
        connectionId: 'c1',
      },
    })
    await settle()
    menuItem('menu-copy-name')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('reports a failure to quote a name', async () => {
    apiStub.quoteIdentifier.mockRejectedValue({
      kind: 'notConnected',
      message: 'gone',
      detail: null,
    })
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        kind: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
      },
    })
    await settle()
    menuItem('menu-copy-name')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('marks the node the user chose', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('activate', {
      key: 'leaf',
      label: 'id',
      kind: 'column',
      icon: 'mdi-table-column',
      loading: false,
      loaded: true,
      connectionId: 'c1',
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'ExplorerTree' }).props('selectedKey')).toBe('leaf')
  })
})

describe('DbExplorer without a filter match', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
  })

  it('closes a branch that was already open', async () => {
    const wrapper = mountWithPlugins(DbExplorer)
    await useConnectionsStore().load()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="tree-row"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="tree-row"]').length).toBeGreaterThan(1)

    await wrapper.find('[data-test="tree-row"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="tree-row"]')).toHaveLength(1)
    expect(root.loaded).toBe(true)
  })
})

describe('DbExplorer menu on a column', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.listDatabases.mockResolvedValue([])
  })

  it('offers no refresh on a column, which holds nothing below it', async () => {
    const wrapper = mountWithPlugins(DbExplorer)
    await useConnectionsStore().load()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 'col',
        label: 'id',
        kind: 'column',
        icon: 'mdi-table-column',
        loading: false,
        loaded: true,
        connectionId: 'c1',
      },
    })
    await settle()

    expect(document.querySelector('[data-test="menu-refresh"]')).toBeNull()
    expect(document.querySelector('[data-test="menu-preview"]')).toBeNull()
    expect(document.querySelector('[data-test="menu-new-query"]')).not.toBeNull()
  })
})
