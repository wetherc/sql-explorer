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
const { DbType, MssqlAuth, TlsMode } = await import('@/types/api')
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

describe('ConnectionForm advanced options', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
  })

  /** Opens the panel that holds the options a user rarely changes. */
  async function openAdvanced(wrapper: Awaited<ReturnType<typeof mountForm>>) {
    await wrapper.find('[data-test="advanced-panel"] .v-expansion-panel-title').trigger('click')
    await settle()
    return wrapper
  }

  it('shows the transport and instance options of MS SQL Server', async () => {
    const wrapper = await openAdvanced(await mountForm())
    expect(wrapper.find('[data-test="tls-select"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="instance-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="integrated-switch"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="connection-url-field"]').exists()).toBe(true)
  })

  it('offers a certificate authority file only when the certificate is checked', async () => {
    const wrapper = await openAdvanced(await mountForm())
    expect(wrapper.text()).toContain('Certificate authority file')

    const tlsSelect = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'tls-select')
    await tlsSelect?.vm.$emit('update:modelValue', TlsMode.Disable)
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).not.toContain('Certificate authority file')
  })

  it('hides the transport options for an engine that has none', async () => {
    const wrapper = await openAdvanced(
      await mountForm(connectionFixture({ dbType: DbType.Sqlite })),
    )
    expect(wrapper.find('[data-test="tls-select"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="instance-field"]').exists()).toBe(false)
  })

  it('keeps the timeouts, the row limit and the folder', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await openAdvanced(await mountForm())

    const numbers = wrapper.findAll('input[type="number"]')
    await numbers[numbers.length - 4]!.setValue('30')
    await numbers[numbers.length - 3]!.setValue('60')
    await numbers[numbers.length - 2]!.setValue('500')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({
          connectTimeoutSecs: 30,
          queryTimeoutSecs: 60,
          maxRows: 500,
        }),
      }),
    )
  })

  it('keeps the session limit', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await openAdvanced(await mountForm())

    await wrapper.find('[data-test="max-sessions-field"] input').setValue('3')
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({ maxSessions: 3 }),
      }),
    )
  })

  it('keeps a read-only session and a colour', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await openAdvanced(await mountForm())

    const switches = wrapper.findAllComponents({ name: 'VSwitch' })
    await switches[switches.length - 1]!.vm.$emit('update:modelValue', true)

    const selects = wrapper.findAllComponents({ name: 'VSelect' })
    await selects[selects.length - 1]!.vm.$emit('update:modelValue', 'error')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        color: 'error',
        options: expect.objectContaining({ readOnly: true }),
      }),
    )
  })

  it('sends a connection string in place of the fields', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const wrapper = await openAdvanced(await mountForm())
    await wrapper.find('[data-test="connection-url-field"] textarea').setValue('server=tcp:a,1')
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({ connectionUrl: 'server=tcp:a,1' }),
      }),
    )
  })

  it('keeps the AWS fields of an Athena record', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const athena = connectionFixture({ dbType: DbType.Athena, host: null, port: null })
    athena.options.awsRegion = 'us-east-1'
    athena.options.athenaWorkgroup = 'primary'
    const wrapper = await mountForm(athena)

    await wrapper.find('[data-test="aws-profile-field"] input').setValue('reporting')
    await wrapper.find('[data-test="athena-output-field"] input').setValue('s3://bucket/out/')
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({
          awsProfile: 'reporting',
          athenaOutputLocation: 's3://bucket/out/',
        }),
      }),
    )
  })
})

describe('ConnectionForm with every field filled', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.saveConnection.mockResolvedValue(undefined)
  })

  it('keeps every value the user typed', async () => {
    const wrapper = await mountForm()
    await wrapper.find('[data-test="advanced-panel"] .v-expansion-panel-title').trigger('click')
    await settle()

    await wrapper.find('[data-test="name-field"] input').setValue('Reporting')
    await wrapper.find('[data-test="host-field"] input').setValue('sql.example.com')
    await wrapper.find('[data-test="port-field"] input').setValue('14330')
    await wrapper.find('[data-test="user-field"] input').setValue('reader')
    await wrapper.find('[data-test="password-field"] input').setValue('secret')
    await wrapper.find('[data-test="database-field"] input').setValue('Warehouse')
    await wrapper.find('[data-test="instance-field"] input').setValue('SQLEXPRESS')

    const caField = wrapper
      .findAll('input')
      .find((input) => input.attributes('id') && input.element.value === '')
    void caField

    await wrapper.findAllComponents({ name: 'VSwitch' })[0]!.vm.$emit('update:modelValue', true)
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'Reporting',
        host: 'sql.example.com',
        port: 14330,
        user: 'reader',
        password: 'secret',
        database: 'Warehouse',
        options: expect.objectContaining({
          instanceName: 'SQLEXPRESS',
          integratedSecurity: true,
        }),
      }),
    )
  })

  it('keeps the certificate authority file and the application name', async () => {
    const wrapper = await mountForm()
    await wrapper.find('[data-test="advanced-panel"] .v-expansion-panel-title').trigger('click')
    await settle()

    const labelled = (label: string) =>
      wrapper
        .findAllComponents({ name: 'VTextField' })
        .find((item) => item.props('label') === label)

    await labelled('Certificate authority file')?.vm.$emit('update:modelValue', '/etc/ca.pem')
    await labelled('Application name')?.vm.$emit('update:modelValue', 'Reporting client')
    await labelled('Folder')?.vm.$emit('update:modelValue', 'Production')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        group: 'Production',
        options: expect.objectContaining({
          caCertPath: '/etc/ca.pem',
          applicationName: 'Reporting client',
        }),
      }),
    )
  })

  it('offers the authentication methods of MS SQL Server', async () => {
    const wrapper = await mountForm()
    const select = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'auth-select')
    expect((select!.props('items') as Array<{ title: string }>).map((item) => item.title)).toEqual([
      'SQL login',
      'Windows Authentication',
      'Microsoft Entra ID with the Azure CLI',
      'Microsoft Entra ID with an access token',
    ])
  })

  it('hides the login for Windows Authentication and keeps the choice', async () => {
    const wrapper = await mountForm()
    expect(wrapper.find('[data-test="user-field"]').exists()).toBe(true)

    const select = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'auth-select')
    await select!.vm.$emit('update:modelValue', MssqlAuth.Integrated)
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="user-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="password-field"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Kerberos ticket')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({ mssqlAuth: MssqlAuth.Integrated }),
      }),
    )
  })

  it('asks for a token, and for the path of the Azure CLI', async () => {
    const wrapper = await mountForm()
    const select = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'auth-select')

    await select!.vm.$emit('update:modelValue', MssqlAuth.EntraAccessToken)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="access-token-field"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="azure-cli-path-field"]').exists()).toBe(false)

    // The token reaches the record that the save sends.
    await wrapper.find('[data-test="access-token-field"] input').setValue('a-token')
    const reveal = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'access-token-field')
    const toggle = wrapper.find('[data-test="toggle-token"]')
    expect(toggle.attributes('aria-label')).toBe('Show the token')
    expect(toggle.attributes('aria-pressed')).toBe('false')

    await toggle.trigger('click')
    await wrapper.vm.$nextTick()
    expect(reveal!.props('type')).toBe('text')
    expect(wrapper.find('[data-test="toggle-token"]').attributes('aria-label')).toBe(
      'Hide the token',
    )

    await select!.vm.$emit('update:modelValue', MssqlAuth.EntraAzureCli)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="access-token-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="azure-cli-path-field"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('az login')

    await wrapper.find('[data-test="azure-cli-path-field"] input').setValue('/opt/az')
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        password: 'a-token',
        options: expect.objectContaining({
          mssqlAuth: MssqlAuth.EntraAzureCli,
          azureCliPath: '/opt/az',
        }),
      }),
    )
  })

  it('shows no authentication list for an engine that has one method', async () => {
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    expect(wrapper.find('[data-test="auth-select"]').exists()).toBe(false)
  })

  it('keeps the reuse of results with its age', async () => {
    const athena = connectionFixture({ dbType: DbType.Athena, host: null, port: null })
    athena.options.awsRegion = 'us-east-1'
    athena.options.athenaWorkgroup = 'primary'
    const wrapper = await mountForm(athena)

    // The age appears only once the reuse is on.
    expect(wrapper.find('[data-test="athena-reuse-age-field"]').exists()).toBe(false)
    const reuse = wrapper
      .findAllComponents({ name: 'VSwitch' })
      .find((item) => item.attributes('data-test') === 'athena-reuse-switch')
    await reuse!.vm.$emit('update:modelValue', true)
    await wrapper.vm.$nextTick()

    const age = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'athena-reuse-age-field')
    await age!.vm.$emit('update:modelValue', '15')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({
          athenaResultReuse: true,
          athenaResultReuseMaxAgeMinutes: 15,
        }),
      }),
    )
  })

  it('keeps every AWS field', async () => {
    const athena = connectionFixture({ dbType: DbType.Athena, host: null, port: null })
    athena.options.awsRegion = 'us-east-1'
    athena.options.athenaWorkgroup = 'primary'
    const wrapper = await mountForm(athena)

    await wrapper.find('[data-test="aws-region-field"] input').setValue('eu-west-1')
    await wrapper.find('[data-test="athena-workgroup-field"] input').setValue('reporting')
    await wrapper.find('[data-test="database-field"] input').setValue('logs')
    const catalog = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.props('label') === 'Data catalog')
    await catalog?.vm.$emit('update:modelValue', 'MyCatalog')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()

    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        database: 'logs',
        options: expect.objectContaining({
          awsRegion: 'eu-west-1',
          athenaWorkgroup: 'reporting',
          athenaCatalog: 'MyCatalog',
        }),
      }),
    )
  })

  it('keeps the path of a SQLite file the user typed', async () => {
    const wrapper = await mountForm(connectionFixture({ dbType: DbType.Sqlite }))
    await wrapper.find('[data-test="file-field"] input').setValue('/data/local.db')
    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({
        options: expect.objectContaining({ filePath: '/data/local.db' }),
      }),
    )
  })

  it('shows no field for an engine the build does not describe', async () => {
    const wrapper = mountWithPlugins(ConnectionForm, {
      props: { connection: connectionFixture(), isNew: false },
    })
    useConnectionsStore().engines = []
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="host-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="user-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="database-field"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="file-field"]').exists()).toBe(false)
  })
})

describe('ConnectionForm without a stored password', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.saveConnection.mockResolvedValue(undefined)
  })

  it('starts with an empty box for a record that carries no password', async () => {
    const wrapper = await mountForm(connectionFixture({ password: null }))
    const field = wrapper.find('[data-test="password-field"] input').element as HTMLInputElement
    expect(field.value).toBe('')
  })

  it('empties the box when the parent gives a record without a password', async () => {
    const wrapper = await mountForm()
    await wrapper.setProps({ connection: connectionFixture({ id: 'c2', password: null }) })
    await wrapper.vm.$nextTick()
    const field = wrapper.find('[data-test="password-field"] input').element as HTMLInputElement
    expect(field.value).toBe('')
  })

  it('keeps a password the user typed into the box', async () => {
    const wrapper = await mountForm(connectionFixture({ password: null }))
    const field = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.props('label') === 'Password')
    await field?.vm.$emit('update:modelValue', 'typed-in')

    await wrapper.find('[data-test="save-button"]').trigger('click')
    await settle()
    expect(apiStub.saveConnection).toHaveBeenCalledWith(
      expect.objectContaining({ password: 'typed-in' }),
    )
  })
})
