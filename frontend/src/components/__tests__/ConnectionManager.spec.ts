import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))

const ConnectionManager = (await import('@/components/ConnectionManager.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useExplorerStore } = await import('@/stores/explorer')
const { ConnectionHealth, DbType } = await import('@/types/api')
type EngineInfo = import('@/types/api').EngineInfo

const engines: EngineInfo[] = [
  {
    dbType: DbType.Mssql,
    label: 'MS SQL Server',
    dialect: 'msSql' as const,
    defaultPort: 1433,
    usesHost: true,
    usesCredentials: true,
    usesDatabase: true,
    usesTls: true,
    usesFile: false,
    usesAws: false,
    supportsSchemas: true,
    supportsIntegratedSecurity: true,
  },
]

async function mountManager(saved = [connectionFixture()]) {
  apiStub.getConnections.mockResolvedValue(saved)
  const wrapper = mountWithPlugins(ConnectionManager)
  const connections = useConnectionsStore()
  connections.engines = engines
  await connections.load()
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('ConnectionManager', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.supportedEngines.mockResolvedValue([])
    apiStub.listDatabases.mockResolvedValue([])
  })

  it('points at the first connection when the list is empty', async () => {
    const wrapper = await mountManager([])
    expect(wrapper.text()).toContain('No connections yet')
    expect(wrapper.find('[data-test="empty-new-connection"]').exists()).toBe(true)
  })

  it('lists a connection with its target and its folder', async () => {
    const wrapper = await mountManager([connectionFixture({ group: 'Production' })])
    expect(wrapper.text()).toContain('Production')
    expect(wrapper.text()).toContain('Server')
    expect(wrapper.text()).toContain('localhost:1433/Sales')
  })

  it('marks each engine with its own icon', async () => {
    const wrapper = await mountManager([
      connectionFixture({ id: 'a', dbType: DbType.Mssql }),
      connectionFixture({ id: 'b', dbType: DbType.Athena }),
      connectionFixture({ id: 'c', dbType: DbType.Postgres }),
      connectionFixture({ id: 'd', dbType: DbType.Mysql }),
      connectionFixture({ id: 'e', dbType: DbType.Sqlite }),
    ])
    for (const icon of [
      'mdi-microsoft',
      'mdi-aws',
      'mdi-elephant',
      'mdi-dolphin',
      'mdi-file-cabinet',
    ]) {
      expect(wrapper.find(`.${icon}`).exists()).toBe(true)
    }
  })

  it('opens a connection and reads its objects', async () => {
    apiStub.connect.mockResolvedValue(infoFixture())
    const wrapper = await mountManager()

    await wrapper.find('[data-test="toggle-connection"]').trigger('click')
    await settle()

    expect(apiStub.connect).toHaveBeenCalled()
    expect(useExplorerStore().roots).toHaveLength(1)
    expect(wrapper.emitted('connected')?.[0]).toEqual(['c1'])
  })

  it('adds no root when the connection cannot open', async () => {
    apiStub.connect.mockRejectedValue({ kind: 'connection', message: 'refused', detail: null })
    const wrapper = await mountManager()
    await wrapper.find('[data-test="toggle-connection"]').trigger('click')
    await settle()
    expect(useExplorerStore().roots).toHaveLength(0)
    expect(wrapper.emitted('connected')).toBeUndefined()
  })

  it('closes a connection that is open', async () => {
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.disconnect.mockResolvedValue(undefined)
    const wrapper = await mountManager()
    useExplorerStore().addRoot('c1')

    await wrapper.find('[data-test="toggle-connection"]').trigger('click')
    await settle()

    expect(apiStub.disconnect).toHaveBeenCalledWith('c1')
    expect(useExplorerStore().roots).toHaveLength(0)
  })

  it('marks the state of a connection', async () => {
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const wrapper = await mountManager()
    expect(wrapper.find('.bg-success').exists()).toBe(true)

    useConnectionsStore().health = { c1: ConnectionHealth.Reconnecting }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.bg-warning').exists()).toBe(true)
  })

  it('selects an open connection for the explorer', async () => {
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const wrapper = await mountManager()
    await wrapper.find('[data-test="connection-item"]').trigger('click')
    expect(useConnectionsStore().selectedId).toBe('c1')
  })

  it('selects nothing when the connection is closed', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="connection-item"]').trigger('click')
    expect(useConnectionsStore().selectedId).toBeNull()
  })

  it('opens the form for a new connection', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="new-connection"]').trigger('click')
    await settle()
    expect(document.body.textContent).toContain('New connection')
  })

  it('opens the form for a record that is edited, with no password in the box', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="connection-menu"]').trigger('click')
    await settle()

    const edit = document.querySelector('[data-test="edit-connection"]') as HTMLElement
    edit.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(document.body.textContent).toContain('Edit Server')
    const password = document.querySelector(
      '[data-test="password-field"] input',
    ) as HTMLInputElement
    expect(password.value).toBe('')
  })

  it('opens the form for a copy of a record', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="connection-menu"]').trigger('click')
    await settle()

    const duplicate = document.querySelector('[data-test="duplicate-connection"]') as HTMLElement
    duplicate.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(document.body.textContent).toContain('New connection')
  })

  it('asks before it removes a record and then removes it', async () => {
    apiStub.deleteConnection.mockResolvedValue(undefined)
    const wrapper = await mountManager()
    useExplorerStore().addRoot('c1')

    await wrapper.find('[data-test="connection-menu"]').trigger('click')
    await settle()
    const remove = document.querySelector('[data-test="delete-connection"]') as HTMLElement
    remove.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(document.body.textContent).toContain('Delete this connection?')
    const confirm = document.querySelector('[data-test="confirm-delete"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.deleteConnection).toHaveBeenCalledWith('c1')
    expect(useExplorerStore().roots).toHaveLength(0)
  })

  it('removes nothing when no record is waiting', async () => {
    const wrapper = await mountManager()
    const view = wrapper.vm as unknown as { confirmDelete?: () => Promise<void> }
    void view
    await wrapper.find('[data-test="connection-menu"]').trigger('click')
    await settle()
    const remove = document.querySelector('[data-test="delete-connection"]') as HTMLElement
    remove.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    const cancel = [...document.querySelectorAll('.v-btn')].find((button) =>
      button.textContent?.includes('Cancel'),
    )
    cancel?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.deleteConnection).not.toHaveBeenCalled()
  })

  it('reads the list again on request', async () => {
    const wrapper = await mountManager()
    apiStub.getConnections.mockClear()
    await wrapper.find('[data-test="refresh-connections"]').trigger('click')
    await settle()
    expect(apiStub.getConnections).toHaveBeenCalled()
  })

  it('closes the form once a record is saved', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    apiStub.supportedEngines.mockResolvedValue([])
    const wrapper = await mountManager()
    await wrapper.find('[data-test="new-connection"]').trigger('click')
    await settle()

    const dialogs = wrapper.findAllComponents({ name: 'VDialog' })
    expect(dialogs[0]?.props('modelValue')).toBe(true)

    await wrapper.findComponent({ name: 'ConnectionForm' }).vm.$emit('saved', 'c9')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents({ name: 'VDialog' })[0]?.props('modelValue')).toBe(false)
  })
})

describe('ConnectionManager dialog state', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.supportedEngines.mockResolvedValue([])
  })

  it('closes the form when it asks to close', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="new-connection"]').trigger('click')
    await settle()

    const open = () =>
      wrapper.findAllComponents({ name: 'VDialog' }).filter((item) => item.props('modelValue'))
    expect(open()).toHaveLength(1)

    await wrapper.findComponent({ name: 'ConnectionForm' }).vm.$emit('close')
    await wrapper.vm.$nextTick()
    expect(open()).toHaveLength(0)
  })

  it('closes the form when the overlay reports it', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="new-connection"]').trigger('click')
    await settle()

    const dialogs = wrapper.findAllComponents({ name: 'VDialog' })
    await dialogs[0]!.vm.$emit('update:modelValue', false)
    await settle()
    expect(dialogs[0]!.props('modelValue')).toBe(false)
  })
})

describe('ConnectionManager delete dialog', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.supportedEngines.mockResolvedValue([])
  })

  it('closes the confirmation when the overlay reports it', async () => {
    const wrapper = await mountManager()
    await wrapper.find('[data-test="connection-menu"]').trigger('click')
    await settle()
    const remove = document.querySelector('[data-test="delete-connection"]') as HTMLElement
    remove.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    const dialogs = wrapper.findAllComponents({ name: 'VDialog' })
    const dialog = dialogs[dialogs.length - 1]!
    expect(dialog.props('modelValue')).toBe(true)
    await dialog.vm.$emit('update:modelValue', false)
    await settle()
    expect(apiStub.deleteConnection).not.toHaveBeenCalled()
  })
})
