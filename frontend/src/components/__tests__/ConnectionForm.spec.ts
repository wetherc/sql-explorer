import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const openDialog = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...args),
  save: vi.fn(),
}))

const ConnectionForm = (await import('@/components/ConnectionForm.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useUiStore } = await import('@/stores/ui')
const { DbType, TlsMode } = await import('@/types/api')
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
  {
    dbType: DbType.Sqlite,
    label: 'SQLite',
    dialect: 'sqlite' as const,
    defaultPort: null,
    usesHost: false,
    usesCredentials: false,
    usesDatabase: false,
    usesTls: false,
    usesFile: true,
    usesAws: false,
    supportsSchemas: false,
    supportsIntegratedSecurity: false,
  },
  {
    dbType: DbType.Athena,
    label: 'AWS Athena',
    dialect: 'athena' as const,
    defaultPort: null,
    usesHost: false,
    usesCredentials: false,
    usesDatabase: true,
    usesTls: false,
    usesFile: false,
    usesAws: true,
    supportsSchemas: false,
    supportsIntegratedSecurity: false,
  },
]

async function mountForm(connection = connectionFixture(), isNew = false) {
  const wrapper = mountWithPlugins(ConnectionForm, { props: { connection, isNew } })
  useConnectionsStore().engines = engines
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('ConnectionForm', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    openDialog.mockReset()
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
  })

  it('shows the fields a network engine uses', async () => {
    const wrapper = await mountForm()
    expect(wrapper.find('[data-test="host-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="port-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="user-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="database-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="file-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="aws-region-field"]').exists()).toBe(false)
  })

  it('shows the file field for SQLite and hides the rest', async () => {
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    expect(wrapper.find('[data-test="file-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="host-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="user-field"]').exists()).toBe(false)
  })

  it('shows the AWS fields for Athena', async () => {
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Athena }))
    expect(wrapper.find('[data-test="aws-region-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="athena-workgroup-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="athena-output-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="host-field"]').exists()).toBe(false)
  })

  it('sets the default port when the engine changes', async () => {
    const wrapper = await mountForm()
    await wrapper.findComponent({ name: 'VSelect' }).vm.$emit('update:modelValue', DbType.Sqlite)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="file-field"]').exists()).toBe(true)
  })

  it('gives a host again when the engine needs one', async () => {
    const wrapper = await mountForm(
      connectionFixture({ dbType: DbType.Sqlite, host: null, port: null }),
    )
    await wrapper.findComponent({ name: 'VSelect' }).vm.$emit('update:modelValue', DbType.Mssql)
    await wrapper.vm.$nextTick()
    const host = wrapper.find('[data-test="host-field"] input').element as HTMLInputElement
    expect(host.value).toBe('localhost')
  })

  it('keeps a host the record already holds', async () => {
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite, host: 'kept' }))
    await wrapper.findComponent({ name: 'VSelect' }).vm.$emit('update:modelValue', DbType.Mssql)
    await wrapper.vm.$nextTick()
    const host = wrapper.find('[data-test="host-field"] input').element as HTMLInputElement
    expect(host.value).toBe('kept')
  })

  it('reports the fields a record still needs', async () => {
    const wrapper = await mountForm(connectionFixture({ name: '' }))
    expect(wrapper.find('[data-test="form-problems"]').text()).toContain('needs a name')
  })

  it('describes each transport setting', async () => {
    const wrapper = await mountForm()
    const advanced = wrapper.findComponent({ name: 'VExpansionPanel' })
    await advanced.find('.v-expansion-panel-title').trigger('click')
    await settle()

    for (const [mode, text] of [
      [TlsMode.VerifyFull, 'identity of the server is checked'],
      [TlsMode.Require, 'a false certificate is accepted'],
      [TlsMode.Prefer, 'continues without encryption'],
      [TlsMode.Disable, 'clear text'],
    ] as const) {
      const tlsSelect = wrapper
        .findAllComponents({ name: 'VSelect' })
        .find((item) => item.attributes('data-test') === 'tls-select')
      await tlsSelect?.vm.$emit('update:modelValue', mode)
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="tls-select"]').text()).toContain(text)
    }
  })

  it('takes the path of a file the user chose', async () => {
    openDialog.mockResolvedValue('/data/local.db')
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    await wrapper.find('[data-test="choose-file"]').trigger('click')
    await settle()
    const field = wrapper.find('[data-test="file-field"] input').element as HTMLInputElement
    expect(field.value).toBe('/data/local.db')
  })

  it('keeps the path when the user closed the dialog', async () => {
    openDialog.mockResolvedValue(null)
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    await wrapper.find('[data-test="choose-file"]').trigger('click')
    await settle()
    const field = wrapper.find('[data-test="file-field"] input').element as HTMLInputElement
    expect(field.value).toBe('')
  })

  it('reports a failure of the file dialog', async () => {
    openDialog.mockRejectedValue({ kind: 'io', message: 'refused', detail: null })
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    await wrapper.find('[data-test="choose-file"]').trigger('click')
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('tests the record the form holds', async () => {
    apiStub.testConnection.mockResolvedValue('The connection works.')
    const wrapper = await mountForm()
    await wrapper.find('[data-test="test-button"]').trigger('click')
    await settle()
    expect(apiStub.testConnection).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'c1', password: 'secret' }),
    )
  })

  it('saves the record and closes the form', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await mountForm()
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalled()
    expect(wrapper.emitted('saved')?.[0]).toEqual(['c1'])
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('keeps the stored password when the box is left empty', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await mountForm(connectionFixture({ password: '' }))
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(expect.objectContaining({ password: null }))
  })

  it('sends an empty password for a new record with no password', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await mountForm(connectionFixture({ password: '' }), true)
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(expect.objectContaining({ password: '' }))
  })

  it('stays open when the record could not be saved', async () => {
    apiStub.saveConnection.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const wrapper = await mountForm()
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('closes without saving on request', async () => {
    const wrapper = await mountForm()
    await wrapper.find('[data-test="cancel-button"]').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(apiStub.saveConnection).not.toHaveBeenCalled()
  })

  it('shows and hides the password', async () => {
    const wrapper = await mountForm()
    const field = wrapper.find('[data-test="password-field"] input')
    expect(field.attributes('type')).toBe('password')
    await wrapper.find('[data-test="password-field"] .mdi-eye').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="password-field"] input').attributes('type')).toBe('text')
  })

  it('reads a new record when the parent gives one', async () => {
    const wrapper = await mountForm()
    await wrapper.setProps({ connection: connectionFixture({ id: 'c2', name: 'Other' }) })
    await wrapper.vm.$nextTick()
    const name = wrapper.find('[data-test="name-field"] input').element as HTMLInputElement
    expect(name.value).toBe('Other')
  })

  it('names the title of a new record and of one that is edited', async () => {
    const newForm = await mountForm(connectionFixture(), true)
    expect(newForm.text()).toContain('New connection')

    const editForm = await mountForm(connectionFixture({ name: 'Reporting' }))
    expect(editForm.text()).toContain('Edit Reporting')

    const blank = await mountForm(connectionFixture({ name: '' }))
    expect(blank.text()).toContain('Edit connection')
  })
})
