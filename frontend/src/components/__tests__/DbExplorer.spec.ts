import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  makeApiStub,
  connectionFixture,
  infoFixture,
  streamed,
} from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const DbExplorer = (await import('@/components/DbExplorer.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { FILTER_DELAY_MS, useExplorerStore } = await import('@/stores/explorer')
const { useQueryStore } = await import('@/stores/query')
const { useSettingsStore } = await import('@/stores/settings')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')
const { RelationType } = await import('@/types/api')

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
    apiStub.listTables.mockResolvedValue([{ name: 'orders', relationType: RelationType.Table }])
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
    expect(wrapper.text()).toContain('No open connections')
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

    // The levels are the connection, the database, the schema, the folder of
    // the tables, the table itself and the folder of its columns.
    for (let level = 0; level < 6; level += 1) {
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
    // The filter of the tree holds the text for a short pause.
    await new Promise((resolve) => setTimeout(resolve, FILTER_DELAY_MS + 20))
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('No matches')
  })

  it('builds the preview statement in the backend and runs it', async () => {
    apiStub.previewQuery.mockResolvedValue('SELECT TOP 1000 * FROM [Sales].[dbo].[orders];')
    apiStub.executeQuery.mockImplementation(
      streamed({
        results: [],
        messages: [],
        rowsAffected: null,
        elapsedMs: 1,
      }),
    )
    apiStub.addHistoryEntry.mockResolvedValue([])

    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    const database = root.children![0]!
    await explorer.expand(database)
    const schema = database.children![0]!
    await explorer.expand(schema)
    const tables = schema.children![0]!
    await explorer.expand(tables)
    explorer.roots = [root]
    await wrapper.vm.$nextTick()

    const table = tables.children![0]!
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
        nodeType: 'table',
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

  it('puts the statement of an object in a new tab without running it', async () => {
    apiStub.scriptObject.mockResolvedValue('SELECT\n    [id]\nFROM [dbo].[orders];')
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 'v',
        label: 'orders',
        nodeType: 'view',
        icon: 'mdi-table-eye',
        loading: false,
        loaded: false,
        connectionId: 'c1',
        database: 'Sales',
        schema: 'dbo',
        table: 'orders',
      },
    })
    await settle()

    menuItem('menu-script-select')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.scriptObject).toHaveBeenCalledWith({
      connectionId: 'c1',
      database: 'Sales',
      schemaName: 'dbo',
      tableName: 'orders',
      parentName: null,
      target: 'view',
      statement: 'select',
    })
    expect(useTabsStore().tabs[0]?.query).toBe('SELECT\n    [id]\nFROM [dbo].[orders];')
    expect(useTabsStore().tabs[0]?.title).toBe('orders (select)')
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
  })

  it('offers the four forms and names a table as a table', async () => {
    apiStub.scriptObject.mockResolvedValue('CREATE TABLE t ();')
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        nodeType: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
        table: 'orders',
      },
    })
    await settle()

    for (const form of ['create', 'select', 'insert', 'update']) {
      expect(menuItem(`menu-script-${form}`)).toBeTruthy()
    }

    menuItem('menu-script-create')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.scriptObject).toHaveBeenCalledWith({
      connectionId: 'c1',
      database: null,
      schemaName: null,
      tableName: 'orders',
      parentName: null,
      target: 'table',
      statement: 'create',
    })
  })

  it('offers the statements that each type of relation takes', async () => {
    apiStub.scriptObject.mockResolvedValue('CREATE SYNONYM [dbo].[alias] FOR [x];')
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    const openFor = async (nodeType: string) => {
      await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
        event: new MouseEvent('contextmenu'),
        node: {
          key: nodeType,
          label: 'r',
          nodeType,
          icon: 'mdi-table',
          loading: false,
          loaded: false,
          connectionId: 'c1',
          table: 'r',
        },
      })
      await settle()
      return ['create', 'select', 'insert', 'update'].filter((form) =>
        menuItem(`menu-script-${form}`),
      )
    }

    expect(await openFor('partitionedTable')).toEqual(['create', 'select', 'insert', 'update'])
    expect(menuItem('menu-properties')).toBeTruthy()
    expect(await openFor('materializedView')).toEqual(['create', 'select'])
    expect(await openFor('foreignTable')).toEqual(['select', 'insert', 'update'])
    expect(await openFor('synonym')).toEqual(['create', 'select'])
    // A synonym reports no facts of its own, so the menu offers no properties.
    expect(menuItem('menu-properties')).toBeNull()
    expect(menuItem('menu-preview')).toBeTruthy()

    menuItem('menu-script-create')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.scriptObject).toHaveBeenCalledWith(
      expect.objectContaining({ tableName: 'r', target: 'synonym', statement: 'create' }),
    )
  })

  it('offers the CREATE statement alone for a trigger and an event', async () => {
    apiStub.scriptObject.mockResolvedValue('CREATE TRIGGER audit AFTER INSERT ON orders;')
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    const openFor = async (objectType: string, table?: string) => {
      await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
        event: new MouseEvent('contextmenu'),
        node: {
          key: objectType,
          label: 'audit',
          nodeType: objectType,
          icon: 'mdi-lightning-bolt',
          loading: false,
          loaded: true,
          connectionId: 'c1',
          database: 'shop',
          table,
        },
      })
      await settle()
      return ['create', 'select', 'insert', 'update'].filter((form) =>
        menuItem(`menu-script-${form}`),
      )
    }

    expect(await openFor('trigger', 'orders')).toEqual(['create'])
    expect(menuItem('menu-preview')).toBeNull()
    expect(menuItem('menu-properties')).toBeNull()
    expect(menuItem('menu-copy-name')).toBeNull()
    menuItem('menu-script-create')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    // A trigger sends its own name, and the name of its table as the parent.
    expect(apiStub.scriptObject).toHaveBeenLastCalledWith({
      connectionId: 'c1',
      database: 'shop',
      schemaName: null,
      tableName: 'audit',
      parentName: 'orders',
      target: 'trigger',
      statement: 'create',
    })
    const tabs = useTabsStore().tabs
    expect(tabs[tabs.length - 1]?.title).toBe('audit (create)')

    expect(await openFor('event')).toEqual(['create'])
    menuItem('menu-script-create')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.scriptObject).toHaveBeenLastCalledWith(
      expect.objectContaining({ tableName: 'audit', parentName: null, target: 'event' }),
    )
  })

  it('reports a failure to build the statement of an object', async () => {
    apiStub.scriptObject.mockRejectedValue({
      category: 'configuration',
      message: 'no column',
      detail: null,
    })
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        nodeType: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
      },
    })
    await settle()

    menuItem('menu-script-insert')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(useUiStore().notices.some((notice) => notice.message.includes('no column'))).toBe(true)
  })

  it('opens the properties of a relation', async () => {
    apiStub.tableDetails.mockResolvedValue({
      facts: [{ name: 'Rows', value: '3' }],
      columns: [],
      indexes: [],
      constraints: [],
    })
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        nodeType: 'table',
        icon: 'mdi-table',
        loading: false,
        loaded: false,
        connectionId: 'c1',
        database: 'Sales',
        schema: 'dbo',
        table: 'orders',
      },
    })
    await settle()

    menuItem('menu-properties')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.tableDetails).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(document.querySelector('[data-test="properties-dialog"]')).toBeTruthy()

    await wrapper.findComponent({ name: 'TableProperties' }).vm.$emit('close')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'TableProperties' }).props('open')).toBe(false)
  })

  it('reports a failure to build the preview statement', async () => {
    apiStub.previewQuery.mockRejectedValue({
      category: 'notConnected',
      message: 'gone',
      detail: null,
    })
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('context', {
      event: new MouseEvent('contextmenu'),
      node: {
        key: 't',
        label: 'orders',
        nodeType: 'table',
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

  it('asks before it closes a connection that a statement runs on', async () => {
    apiStub.disconnect.mockResolvedValue(undefined)
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    const state = useQueryStore().stateFor('t1')
    state.running = true
    state.requestConnectionId = 'c1'
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    menuItem('menu-disconnect')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.disconnect).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('One statement is running on this connection')

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.disconnect).toHaveBeenCalledWith('c1')
    expect(useExplorerStore().roots).toHaveLength(0)
  })

  it('keeps the connection of the tree when the question is refused', async () => {
    apiStub.disconnect.mockResolvedValue(undefined)
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    const state = useQueryStore().stateFor('t1')
    state.running = true
    state.requestConnectionId = 'c1'
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    menuItem('menu-disconnect')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.disconnect).not.toHaveBeenCalled()
    expect(useExplorerStore().roots).toHaveLength(1)
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
        nodeType: 'table',
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

  it('warns when the host offers no clipboard', async () => {
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
        nodeType: 'view',
        icon: 'mdi-table-eye',
        loading: false,
        loaded: false,
        connectionId: 'c1',
      },
    })
    await settle()
    menuItem('menu-copy-name')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    const notices = useUiStore().notices
    expect(notices.some((notice) => notice.level === 'success')).toBe(false)
    expect(notices.some((notice) => notice.message.includes("wasn't copied"))).toBe(true)
  })

  it('reads a failed branch again from the Retry row', async () => {
    const wrapper = await mountExplorer()
    const explorer = useExplorerStore()
    explorer.addRoot('c1')
    await wrapper.vm.$nextTick()
    const spy = vi.spyOn(explorer, 'expand').mockResolvedValue(undefined)
    const root = explorer.roots[0]!
    wrapper.findComponent({ name: 'ExplorerTree' }).vm.$emit('retry', root)
    await settle()
    expect(spy).toHaveBeenCalledWith(root)
  })

  it('moves the focus into the menu and back to the row', async () => {
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()

    await openMenu(wrapper, 0)
    await settle()
    const first = document
      .querySelector('[data-test="menu-new-query"]')!
      .closest('.v-list')!
      .querySelector('.v-list-item')
    expect(document.activeElement).toBe(first)

    ;(document.activeElement as HTMLElement).blur()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await settle()
    await settle()
    expect(document.activeElement).toBe(wrapper.find('[data-test="tree-row"]').element)
  })

  it('leaves the focus alone when the menu shuts into another element', async () => {
    const wrapper = await mountExplorer()
    useExplorerStore().addRoot('c1')
    await wrapper.vm.$nextTick()
    await openMenu(wrapper, 0)
    await settle()
    const filter = wrapper.find('input').element as HTMLInputElement
    filter.focus()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await settle()
    await settle()
    expect(document.activeElement).toBe(filter)
  })

  it('reports a failure to quote a name', async () => {
    apiStub.quoteIdentifier.mockRejectedValue({
      category: 'notConnected',
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
        nodeType: 'table',
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
      nodeType: 'column',
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
        nodeType: 'column',
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
