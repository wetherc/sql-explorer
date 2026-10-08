import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ResultPane } from '@/stores/query'
import type { ResultStreamHandlers } from '@/lib/results'
import type { TextEncoding } from '@/types/api'
import {
  makeApiStub,
  connectionFixture,
  infoFixture,
  streamed,
} from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { monaco } = await import('@/plugins/monaco')
const { tabActions } = await import('@/lib/commands')
const QueryView = (await import('@/components/QueryView.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useFilesStore } = await import('@/stores/files')
const { useLayoutStore } = await import('@/stores/layout')
const { useQueryStore } = await import('@/stores/query')
const { useSettingsStore } = await import('@/stores/settings')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')

const response = {
  results: [
    {
      columns: [{ name: 'n', typeName: 'int' }],
      rows: [[1]],
      truncated: false,
    },
  ],
  messages: [{ level: 'info' as const, text: '1 row returned.', detail: null }],
  rowsAffected: null,
  elapsedMs: 8,
}

/** The notice the view raised last. */
function lastNotice() {
  const notices = useUiStore().notices
  return notices[notices.length - 1]
}

/** The result that the grid hands over when it asks for an export. */
function exported() {
  return {
    columns: [{ name: 'n', typeName: 'int' }],
    rows: [[1]],
    truncated: false,
  }
}

/**
 * Puts an editor with a model in place of the stub, so that the format
 * action has a text to work on. Returns the spy that records the writes.
 */
function editorWithText(text: string) {
  const executeEdits = vi.fn()
  vi.mocked(monaco.editor.create).mockReturnValue({
    getValue: vi.fn(() => text),
    setValue: vi.fn(),
    getModel: vi.fn(() => ({
      getValue: () => text,
      getValueInRange: vi.fn(() => text),
      getFullModelRange: vi.fn(() => ({ whole: true })),
      getOffsetAt: vi.fn(() => 0),
      getPositionAt: vi.fn(() => ({ lineNumber: 1, column: 1 })),
      getWordUntilPosition: vi.fn(() => ({ startColumn: 1, endColumn: 1 })),
    })),
    getSelection: vi.fn(() => null),
    getPosition: vi.fn(() => null),
    onDidChangeModelContent: vi.fn(),
    addAction: vi.fn(),
    updateOptions: vi.fn(),
    executeEdits,
    focus: vi.fn(),
    dispose: vi.fn(),
    saveViewState: vi.fn(() => null),
    restoreViewState: vi.fn(),
  } as unknown as ReturnType<typeof monaco.editor.create>)
  return executeEdits
}

/** Mounts the view and runs one statement, so a grid is on show. */
async function mountedWithResult() {
  apiStub.executeQuery.mockImplementation(streamed(response))
  const wrapper = await mountView()
  await wrapper.find('[data-test="run-button"]').trigger('click')
  await settle()
  await wrapper.vm.$nextTick()
  return wrapper
}

async function mountView(
  query = 'SELECT 1',
  filePath: string | null = null,
  encoding: TextEncoding = 'utf8',
) {
  const wrapper = mountWithPlugins(QueryView, {
    props: {
      tab: {
        id: 't1',
        title: 'Query 1',
        query,
        connectionId: 'c1',
        dirty: false,
        params: [],
        filePath,
        encoding,
      },
    },
  })
  const connections = useConnectionsStore()
  await connections.load()
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('QueryView', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    // Most statements of these tests hold no parameter.
    apiStub.queryParameters.mockResolvedValue([])
  })

  it('names a connection that is not open in place of its identifier', async () => {
    apiStub.listActiveConnections.mockResolvedValue([])
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't1',
          title: 'Query 1',
          query: 'SELECT 1',
          connectionId: 'gone',
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    const connections = useConnectionsStore()
    await connections.load()
    await wrapper.vm.$nextTick()

    const select = wrapper.findComponent({ name: 'VSelect' })
    expect(select.props('items')).toEqual([
      { title: 'Deleted connection (not open)', value: 'gone' },
    ])
  })

  it('names a saved connection that is closed as one that is not open', async () => {
    apiStub.listActiveConnections.mockResolvedValue([])
    const wrapper = await mountView()
    const select = wrapper.findComponent({ name: 'VSelect' })
    expect(select.props('items')).toEqual([{ title: 'Server (not open)', value: 'c1' }])
  })

  it('marks a message that carries a warning and shows what the server said', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response,
        messages: [
          {
            level: 'warning' as const,
            text: 'value out of range',
            detail: 'WARNING · 22003',
          },
        ],
      }),
    )
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.find('[data-test="messages-tab"]').trigger('click')
    await wrapper.vm.$nextTick()

    const line = wrapper.find('[data-test="query-message"]')
    expect(line.classes()).toContain('message-warning')
    expect(line.text()).toContain('value out of range')
    expect(line.text()).toContain('WARNING · 22003')
  })

  it('sends the statement to the backend when Run is pressed', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ connectionId: 'c1', query: 'SELECT 1' }),
      expect.anything(),
    )
  })

  it('runs the statement or the script when the editor reports a run key', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    const editor = wrapper.findComponent({ name: 'SqlEditor' })

    editor.vm.$emit('run-statement')
    await settle()
    editor.vm.$emit('run-all')
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledTimes(2)
    expect(apiStub.executeQuery).toHaveBeenLastCalledWith(
      expect.objectContaining({ connectionId: 'c1', query: 'SELECT 1' }),
      expect.anything(),
    )
  })

  it('asks for a value before it runs a statement that holds a name', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT * FROM t WHERE a = :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    // The dialog stands open, so nothing reached the backend.
    expect(apiStub.executeQuery).not.toHaveBeenCalled()

    const field = document.querySelector(
      '[data-test="parameter-value-id"] input',
    ) as HTMLInputElement
    field.value = '7'
    field.dispatchEvent(new Event('input'))
    await settle()
    ;(document.querySelector('[data-test="parameters-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { id: '7' } }),
      expect.anything(),
    )
  })

  it('brings the focus to the dialog when a second run asks for the same values', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT * FROM t WHERE a = :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    const field = document.querySelector('[data-test="parameter-value-id"] input') as HTMLElement
    expect(field).not.toBeNull()
    ;(document.activeElement as HTMLElement)?.blur()

    // A second run arrives while the dialog still waits for the values.
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(document.activeElement).toBe(field)
    // The dialog says what it needs, so no notice repeats it.
    expect(useUiStore().notices).toHaveLength(0)
  })

  it('runs a second time without the dialog', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't1',
          title: 'Query 1',
          query: 'SELECT :id',
          connectionId: 'c1',
          dirty: false,
          params: [{ name: 'id', valueType: 'number', text: '7' }],
        },
      },
    })
    const connections = useConnectionsStore()
    await connections.load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { id: 7 } }),
      expect.anything(),
    )
  })

  it('runs nothing when the user closes the parameter dialog', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    ;(document.querySelector('[data-test="parameters-cancel"]') as HTMLElement).click()
    await settle()
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
  })

  it('opens the parameter dialog on its own and keeps the values', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="parameters-button"]').trigger('click')
    await settle()
    const field = document.querySelector(
      '[data-test="parameter-value-id"] input',
    ) as HTMLInputElement
    field.value = '9'
    field.dispatchEvent(new Event('input'))
    await settle()
    ;(document.querySelector('[data-test="parameters-confirm"]') as HTMLElement).click()
    await settle()

    // The dialog closed on its own, so no statement ran.
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
  })

  it('sends an empty value when the user chooses that form', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const select = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => String(item.attributes('data-test')).startsWith('parameter-type'))!
    await select.vm.$emit('update:modelValue', 'null')
    await settle()
    ;(document.querySelector('[data-test="parameters-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { id: null } }),
      expect.anything(),
    )
  })

  it('refuses a number that it cannot read and holds the confirm button', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const typeSelect = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => String(item.attributes('data-test')).startsWith('parameter-type'))!
    await typeSelect.vm.$emit('update:modelValue', 'number')
    await settle()

    const field = document.querySelector(
      '[data-test="parameter-value-id"] input',
    ) as HTMLInputElement
    field.value = 'two'
    field.dispatchEvent(new Event('input'))
    await settle()

    expect(document.body.textContent).toContain('Enter a number.')
    const confirm = document.querySelector('[data-test="parameters-confirm"]') as HTMLElement
    expect(confirm.hasAttribute('disabled')).toBe(true)

    // A number that the box accepts opens the way again.
    field.value = '12'
    field.dispatchEvent(new Event('input'))
    await settle()
    expect(document.body.textContent).not.toContain('Enter a number.')
    ;(document.querySelector('[data-test="parameters-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { id: 12 } }),
      expect.anything(),
    )
  })

  it('offers the two words of a true or false value', async () => {
    apiStub.queryParameters.mockResolvedValue(['flag'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT :flag')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const typeSelect = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => String(item.attributes('data-test')).startsWith('parameter-type'))!
    await typeSelect.vm.$emit('update:modelValue', 'boolean')
    await settle()

    // The value becomes a box of two words, which starts at false.
    const value = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'parameter-value-flag')!
    expect(value.props('items')).toEqual(['true', 'false'])
    expect(value.props('modelValue')).toBe('false')

    await value.vm.$emit('update:modelValue', 'true')
    await settle()
    ;(document.querySelector('[data-test="parameters-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { flag: true } }),
      expect.anything(),
    )
  })

  it('keeps a word that the true or false form already accepts', async () => {
    apiStub.queryParameters.mockResolvedValue(['flag'])
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT :flag')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    const field = document.querySelector(
      '[data-test="parameter-value-flag"] input',
    ) as HTMLInputElement
    field.value = 'true'
    field.dispatchEvent(new Event('input'))
    await settle()

    const typeSelect = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => String(item.attributes('data-test')).startsWith('parameter-type'))!
    await typeSelect.vm.$emit('update:modelValue', 'boolean')
    await settle()

    const value = wrapper
      .findAllComponents({ name: 'VSelect' })
      .find((item) => item.attributes('data-test') === 'parameter-value-flag')!
    expect(value.props('modelValue')).toBe('true')
  })

  it('drops the names of an older read that answers after a newer read', async () => {
    let answerOld: (names: string[]) => void = () => undefined
    apiStub.queryParameters
      .mockImplementationOnce(() => new Promise((resolve) => (answerOld = resolve)))
      .mockResolvedValue(['city'])
    const wrapper = await mountView('SELECT :id')
    await wrapper.setProps({ tab: { ...wrapper.props('tab'), id: 't2', query: 'SELECT :city' } })
    await settle()
    answerOld(['id'])
    await settle()
    expect(wrapper.find('[data-test="parameter-chip-city"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="parameter-chip-id"]').exists()).toBe(false)
  })

  it('names the parameters of the statement in a bar above the editor', async () => {
    apiStub.queryParameters.mockResolvedValue(['id', 'city'])
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't1',
          title: 'Query 1',
          query: 'SELECT * FROM t WHERE a = :id AND b = :city',
          connectionId: 'c1',
          dirty: false,
          params: [{ name: 'id', valueType: 'number', text: '7' }],
        },
      },
    })
    await useConnectionsStore().load()
    await settle()

    const bar = wrapper.find('[data-test="parameter-bar"]')
    expect(bar.exists()).toBe(true)
    expect(wrapper.find('[data-test="parameter-chip-id"]').text()).toBe(':id = 7')
    // A value that is still missing stands out.
    const missing = wrapper
      .findAllComponents({ name: 'VChip' })
      .find((chip) => chip.attributes('data-test') === 'parameter-chip-city')!
    expect(missing.text()).toBe(':city = unset')
    expect(missing.props('color')).toBe('warning')
  })

  it('opens the values dialog from a chip of the bar', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')
    await settle()

    await wrapper.find('[data-test="parameter-chip-id"]').trigger('click')
    await settle()

    expect(document.querySelector('[data-test="parameter-value-id"]')).not.toBeNull()
  })

  it('waits for the writing to stop before it reads the names again', async () => {
    vi.useFakeTimers()
    try {
      apiStub.queryParameters.mockResolvedValue(['id'])
      const wrapper = await mountView('SELECT :id')
      await vi.runAllTimersAsync()
      const reads = apiStub.queryParameters.mock.calls.length

      // Three letters arrive one after the other inside the wait.
      await wrapper.setProps({ tab: { ...wrapper.props('tab'), query: 'SELECT :i' } })
      await wrapper.setProps({ tab: { ...wrapper.props('tab'), query: 'SELECT :id' } })
      await wrapper.setProps({ tab: { ...wrapper.props('tab'), query: 'SELECT :id2' } })
      await vi.advanceTimersByTimeAsync(299)
      expect(apiStub.queryParameters.mock.calls.length).toBe(reads)

      await vi.advanceTimersByTimeAsync(1)
      expect(apiStub.queryParameters.mock.calls.length).toBe(reads + 1)
      wrapper.unmount()
    } finally {
      vi.useRealTimers()
    }
  })

  it('takes its wait away when the view goes', async () => {
    vi.useFakeTimers()
    try {
      apiStub.queryParameters.mockResolvedValue(['id'])
      const wrapper = await mountView('SELECT :id')
      await vi.runAllTimersAsync()
      const reads = apiStub.queryParameters.mock.calls.length

      // The view goes while the wait of the last change still runs.
      await wrapper.setProps({ tab: { ...wrapper.props('tab'), query: 'SELECT :id2' } })
      wrapper.unmount()
      await vi.advanceTimersByTimeAsync(400)

      expect(apiStub.queryParameters.mock.calls.length).toBe(reads)
    } finally {
      vi.useRealTimers()
    }
  })

  it('holds the bar back when the names cannot be read', async () => {
    apiStub.queryParameters.mockRejectedValue({ category: 'database', message: 'no', detail: null })
    const wrapper = await mountView('SELECT :id')
    await settle()

    expect(wrapper.find('[data-test="parameter-bar"]').exists()).toBe(false)
    // The bar is a help alone, so it raises no alarm of its own.
    expect(useUiStore().notices).toHaveLength(0)
  })

  it('says how a parameter is written', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="parameters-button"]').trigger('click')
    await settle()

    expect(document.querySelector('[data-test="parameters-help"]')?.textContent).toContain(
      'by writing',
    )
  })

  it('closes the parameter dialog when the overlay reports it', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')
    await wrapper.find('[data-test="parameters-button"]').trigger('click')
    await settle()

    const dialog = wrapper
      .findAllComponents({ name: 'VDialog' })
      .find((item) => item.props('modelValue'))!
    await dialog.vm.$emit('update:modelValue', false)
    await settle()
    expect(dialog.props('modelValue')).toBe(false)
  })

  it('reports a statement that holds no parameter', async () => {
    apiStub.queryParameters.mockResolvedValue([])
    const wrapper = await mountView()
    await wrapper.find('[data-test="parameters-button"]').trigger('click')
    await settle()
    expect(useUiStore().notices[0]?.message).toBe('This statement has no parameters.')
  })

  it('reports a failure to read the names of the parameters', async () => {
    apiStub.queryParameters.mockRejectedValue(new Error('no reader'))
    const wrapper = await mountView('SELECT :id')

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    expect(apiStub.executeQuery).not.toHaveBeenCalled()

    await wrapper.find('[data-test="parameters-button"]').trigger('click')
    await settle()
    expect(useUiStore().notices.length).toBe(2)
  })

  it('sends the values of the parameters with a plan', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    apiStub.explainQuery.mockResolvedValue(response)
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't1',
          title: 'Query 1',
          query: 'SELECT :id',
          connectionId: 'c1',
          dirty: false,
          params: [{ name: 'id', valueType: 'text', text: 'a' }],
        },
      },
    })
    const connections = useConnectionsStore()
    await connections.load()
    await wrapper.vm.$nextTick()

    wrapper.vm.readPlan('estimated')
    await settle()
    expect(apiStub.explainQuery).toHaveBeenCalledWith(
      expect.objectContaining({ queryParams: { id: 'a' } }),
    )
  })

  it('runs the statement from the menu of the Run button, with or without all rows', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()

    await wrapper.find('[data-test="run-menu-button"]').trigger('click')
    await wrapper.vm.$nextTick()
    ;(document.querySelector('[data-test="run-menu-run"]') as HTMLElement).click()
    await settle()
    expect(apiStub.executeQuery.mock.calls[0]![0]).toMatchObject({
      query: 'SELECT 1',
      spill: undefined,
    })

    await wrapper.find('[data-test="run-menu-button"]').trigger('click')
    await wrapper.vm.$nextTick()
    ;(document.querySelector('[data-test="run-menu-keep-rows"]') as HTMLElement).click()
    await settle()
    expect(apiStub.executeQuery.mock.calls[1]![0]).toMatchObject({
      query: 'SELECT 1',
      spill: { maxRows: expect.any(Number), maxBytes: expect.any(Number) },
    })
  })

  it('reads the estimated plan from the menu', async () => {
    apiStub.explainQuery.mockResolvedValue(response)
    const wrapper = await mountView()

    await wrapper.find('[data-test="plan-button"]').trigger('click')
    await wrapper.vm.$nextTick()
    const item = document.querySelector('[data-test="plan-estimated"]') as HTMLElement
    item.click()
    await settle()

    expect(apiStub.explainQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1', mode: 'estimated' }),
    )
  })

  it('asks before it runs the statement for an actual plan', async () => {
    apiStub.explainQuery.mockResolvedValue(response)
    const wrapper = await mountView()

    await wrapper.find('[data-test="plan-button"]').trigger('click')
    await wrapper.vm.$nextTick()
    ;(document.querySelector('[data-test="plan-actual"]') as HTMLElement).click()
    await wrapper.vm.$nextTick()
    expect(apiStub.explainQuery).not.toHaveBeenCalled()

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.click()
    await settle()
    expect(apiStub.explainQuery).toHaveBeenCalledWith(expect.objectContaining({ mode: 'actual' }))
  })

  it('closes the plan question without a run', async () => {
    const wrapper = await mountView()
    await wrapper.find('[data-test="plan-button"]').trigger('click')
    await wrapper.vm.$nextTick()
    ;(document.querySelector('[data-test="plan-actual"]') as HTMLElement).click()
    await settle()

    const confirm = wrapper
      .findAllComponents({ name: 'ConfirmDialog' })
      .find((item) => item.props('open'))!
    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.explainQuery).not.toHaveBeenCalled()
    expect(confirm.props('open')).toBe(false)
  })

  it('holds no plan button for an engine that reads no plan', async () => {
    const info = infoFixture()
    apiStub.listActiveConnections.mockResolvedValue([
      { ...info, capabilities: { ...info.capabilities, supportsExplain: false } },
    ])
    const wrapper = await mountView()
    expect(wrapper.find('[data-test="plan-button"]').exists()).toBe(false)
  })

  it('refuses a plan without a connection', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't1',
          title: 'Query 1',
          query: 'SELECT 1',
          connectionId: null,
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    await wrapper.vm.$nextTick()

    wrapper.vm.readPlan('estimated')
    await settle()
    expect(apiStub.explainQuery).not.toHaveBeenCalled()
    const notices = useUiStore().notices
    expect(notices[notices.length - 1]?.message).toBe('Choose a connection to see the plan.')
  })

  it('sends the whole script when Run all is pressed', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT 1;\nSELECT 2')

    await wrapper.find('[data-test="run-all-button"]').trigger('click')
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
      expect.anything(),
    )
  })

  it('shows the result after a statement runs', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(1)
    expect(wrapper.text()).toContain('Result 1 (1 row)')
  })

  it('shows the reason a statement failed', async () => {
    apiStub.executeQuery.mockRejectedValue({
      category: 'database',
      message: 'no such column: bad',
      detail: 'line 1',
    })
    const wrapper = await mountView('SELECT bad')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    const error = wrapper.find('[data-test="query-error"]')
    expect(error.text()).toContain('no such column: bad')
    expect(error.text()).toContain('line 1')
  })

  it('names the category, gives advice and offers to copy a failure', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    apiStub.executeQuery.mockRejectedValue({
      category: 'timeout',
      message: 'took too long',
      detail: 'after 30 s',
    })
    const wrapper = await mountView('SELECT 1')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const error = wrapper.find('[data-test="query-error"]')
    expect(error.find('.mdi-timer-alert-outline').exists()).toBe(true)
    expect(wrapper.find('[data-test="query-error-advice"]').text()).toContain('timeout')
    // The backend named no place, so there is no line to go to.
    expect(wrapper.find('[data-test="query-error-goto"]').exists()).toBe(false)

    await wrapper.find('[data-test="query-error-copy"]').trigger('click')
    await settle()
    expect(writeText).toHaveBeenCalledWith('took too long\nafter 30 s')
    expect(lastNotice()?.message).toBe('Error copied to the clipboard.')

    writeText.mockRejectedValue(new Error('denied'))
    await wrapper.find('[data-test="query-error-copy"]').trigger('click')
    await settle()
    expect(lastNotice()?.message).toBe("Couldn't copy the error to the clipboard.")
  })

  it('gives no advice for a failure that needs none', async () => {
    apiStub.executeQuery.mockRejectedValue({ category: 'database', message: 'bad', detail: null })
    const wrapper = await mountView('SELECT 1')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    expect(wrapper.find('[data-test="query-error-advice"]').exists()).toBe(false)
  })

  it('marks the place of a failure and moves the cursor there', async () => {
    const setPosition = vi.fn()
    const revealLineInCenter = vi.fn()
    const focus = vi.fn()
    const model = {
      getValue: () => 'SELECT\n  bad',
      getLineCount: () => 2,
      getLineMaxColumn: () => 6,
      getValueInRange: () => '',
      getOffsetAt: () => 0,
      getPositionAt: () => ({ lineNumber: 1, column: 1 }),
    }
    let changed: () => void = () => {}
    vi.mocked(monaco.editor.create).mockReturnValue({
      getValue: () => 'SELECT\n  bad',
      getModel: () => model,
      getSelection: () => null,
      getPosition: () => null,
      onDidChangeModelContent: vi.fn((listener: () => void) => {
        changed = listener
      }),
      addAction: vi.fn(),
      updateOptions: vi.fn(),
      setPosition,
      revealLineInCenter,
      focus,
      dispose: vi.fn(),
      saveViewState: vi.fn(() => null),
      restoreViewState: vi.fn(),
    } as unknown as ReturnType<typeof monaco.editor.create>)
    vi.mocked(monaco.editor.setModelMarkers).mockClear()
    apiStub.executeQuery.mockRejectedValue({
      category: 'database',
      message: 'no column bad',
      detail: null,
      line: 2,
      column: 3,
    })
    const wrapper = await mountView('SELECT\n  bad')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(model, 'sql-explorer', [
      expect.objectContaining({ startLineNumber: 2, startColumn: 3, message: 'no column bad' }),
    ])
    const goto = wrapper.find('[data-test="query-error-goto"]')
    expect(goto.text()).toBe('Go to line 2')
    await goto.trigger('click')
    expect(setPosition).toHaveBeenCalledWith({ lineNumber: 2, column: 3 })
    expect(revealLineInCenter).toHaveBeenCalledWith(2)

    // The first edit removes the place, so a later remount shows no mark.
    changed()
    await settle()
    expect(useQueryStore().stateFor('t1').errorLocation).toBeNull()
    expect(wrapper.find('[data-test="query-error-goto"]').exists()).toBe(false)

    // A plan failure names a line of the text with the plan keyword in
    // front, so it gives no place in the editor.
    apiStub.explainQuery.mockRejectedValue({
      category: 'database',
      message: 'bad plan',
      detail: null,
      line: 1,
      column: 9,
    })
    wrapper.vm.readPlan('estimated' as never)
    await settle()
    expect(apiStub.explainQuery).toHaveBeenCalled()
    expect(useQueryStore().stateFor('t1').error?.message).toBe('bad plan')
    expect(wrapper.find('[data-test="query-error-goto"]').exists()).toBe(false)
  })

  it('disables the run buttons while a statement runs and reports a stop', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockReturnValue(new Promise(() => {}))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(wrapper.find('[data-test="run-button"]').attributes('disabled')).toBeDefined()
    expect(wrapper.find('[data-test="run-all-button"]').attributes('disabled')).toBeDefined()

    await wrapper.find('[data-test="cancel-button"]').trigger('click')
    await settle()
    const stop = wrapper.find('[data-test="cancel-button"]')
    expect(stop.text()).toBe('Stopping…')
    expect(stop.attributes('disabled')).toBeDefined()

    release(response)
    await settle()
  })

  it('says why a run goes on past the row limit', async () => {
    let release: (value: unknown) => void = () => {}
    let handlers: ResultStreamHandlers | null = null
    apiStub.executeQuery.mockImplementation((_request: unknown, given: ResultStreamHandlers) => {
      handlers = given
      return new Promise((resolve) => {
        release = resolve
      })
    })
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    expect(wrapper.find('[data-test="reading-past-limit"]').exists()).toBe(false)

    handlers!.onReadingPastLimit!()
    await wrapper.vm.$nextTick()
    const status = wrapper.find('[data-test="reading-past-limit"]')
    expect(status.attributes('role')).toBe('status')
    expect(status.text()).toBe(
      "Still reading rows past the limit. The server can't end this batch early.",
    )

    release(response)
    await settle()
    expect(wrapper.find('[data-test="reading-past-limit"]').exists()).toBe(false)
  })

  it('offers a Stop button only while a statement runs', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockResolvedValue(undefined)
    const wrapper = await mountView()
    expect(wrapper.find('[data-test="cancel-button"]').exists()).toBe(false)

    wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="cancel-button"]').trigger('click')
    expect(apiStub.cancelQuery).toHaveBeenCalled()

    release(response)
    await settle()
  })

  it('refuses to run without a connection', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't2',
          title: 'Query 2',
          query: 'SELECT 1',
          connectionId: null,
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    const view = wrapper.vm as unknown as { runAll: () => void }
    view.runAll()
    await settle()
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
    expect(useUiStore().notices[0]?.message).toContain('Choose a connection')
  })

  it('writes the text of the editor back into the tab', async () => {
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: false,
        params: [],
        filePath: null,
        encoding: 'utf8' as const,
      },
    ]
    await wrapper.findComponent({ name: 'SqlEditor' }).vm.$emit('update:modelValue', 'SELECT 2')
    expect(tabs.tabs[0]?.query).toBe('SELECT 2')
  })

  it('changes the connection of the tab', async () => {
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: false,
        params: [],
        filePath: null,
        encoding: 'utf8' as const,
      },
    ]
    await wrapper.findComponent({ name: 'VSelect' }).vm.$emit('update:modelValue', 'c2')
    expect(tabs.tabs[0]?.connectionId).toBe('c2')
  })

  it('asks before a change of the connection stops a running statement', async () => {
    apiStub.releaseSession.mockResolvedValue(undefined)
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: false,
        params: [],
        filePath: null,
        encoding: 'utf8' as const,
      },
    ]
    useQueryStore().stateFor('t1').running = true
    const select = wrapper.findComponent({ name: 'VSelect' })

    await select.vm.$emit('update:modelValue', 'c2')
    await settle()
    expect(tabs.tabs[0]?.connectionId).toBe('c1')
    expect(document.body.textContent).toContain('Change the connection?')
    ;(document.querySelector('[data-test="confirm-cancel"]') as HTMLElement).click()
    await settle()
    expect(tabs.tabs[0]?.connectionId).toBe('c1')

    await select.vm.$emit('update:modelValue', 'c2')
    await settle()
    ;(document.querySelector('[data-test="confirm-accept"]') as HTMLElement).click()
    await settle()
    expect(tabs.tabs[0]?.connectionId).toBe('c2')
  })

  it('writes a result to the file the user chose', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    apiStub.saveTextFile.mockResolvedValue('/tmp/out.csv')

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()

    expect(apiStub.saveTextFile).toHaveBeenCalledWith(
      expect.objectContaining({ extension: 'csv', contents: '\ufeffn\r\n1\r\n' }),
    )
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('writes a result as JSON', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    apiStub.saveTextFile.mockResolvedValue('/tmp/out.json')

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'json', exported())
    await settle()
    expect(apiStub.saveTextFile).toHaveBeenCalledWith(
      expect.objectContaining({ extension: 'json', contents: '[\n  {\n    "n": 1\n  }\n]' }),
    )
  })

  it('writes nothing when the user closed the file dialog', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    apiStub.saveTextFile.mockResolvedValue(null)

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(false)
  })

  it('reports a failure to write a file', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    apiStub.saveTextFile.mockRejectedValue({ category: 'io', message: 'read only', detail: null })

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('warns when the rows did not reach the clipboard', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('copy-failed', 'denied')
    const notice = lastNotice()
    expect(notice?.message).toBe("Couldn't copy to the clipboard.")
    expect(notice?.detail).toBe('denied')
  })

  it('notes that the rows reached the clipboard', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('copied', 'n\n1')
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('writes the statement back to the file that the tab came from', async () => {
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/report.sql', encoding: 'utf8' })
    const wrapper = await mountView('SELECT 1', '/data/report.sql')
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'report.sql',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: '/data/report.sql',
        encoding: 'utf8' as const,
      },
    ]

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()

    expect(apiStub.saveStatementFile).toHaveBeenCalledWith(
      expect.objectContaining({ path: '/data/report.sql', contents: 'SELECT 1', encoding: 'utf8' }),
    )
    expect(tabs.tabs[0]).toMatchObject({ filePath: '/data/report.sql', dirty: false })
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('follows the file that the user chose when the file of the tab was out of reach', async () => {
    // The backend opens the save dialog for a file outside every open folder,
    // and the user can choose another name there.
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/copy.sql', encoding: 'utf8' })
    const wrapper = await mountView('SELECT 1', '/data/report.sql')
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'report.sql',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: '/data/report.sql',
        encoding: 'utf8' as const,
      },
    ]

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()

    expect(tabs.tabs[0]).toMatchObject({
      filePath: '/data/copy.sql',
      title: 'copy.sql',
      dirty: false,
    })
    expect(lastNotice()?.message).toBe('Saved copy.sql.')
  })

  it('keeps the mark of a tab that changed while the write ran', async () => {
    let finish: () => void = () => {}
    apiStub.saveStatementFile.mockReturnValue(
      new Promise((resolve) => {
        finish = () => resolve({ path: '/data/report.sql', encoding: 'utf8' })
      }),
    )
    const wrapper = await mountView('SELECT 1', '/data/report.sql')
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'report.sql',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: '/data/report.sql',
        encoding: 'utf8' as const,
      },
    ]

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    tabs.setQuery('t1', 'SELECT 12')
    finish()
    await settle()

    expect(apiStub.saveStatementFile).toHaveBeenCalledWith(
      expect.objectContaining({ path: '/data/report.sql', contents: 'SELECT 1' }),
    )
    expect(tabs.tabs[0]?.dirty).toBe(true)
    tabs.setQuery('t1', 'SELECT 1')
    expect(tabs.tabs[0]?.dirty).toBe(false)
  })

  it('asks for a path when the tab holds no file, and keeps that path', async () => {
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/daily.sql', encoding: 'utf8' })
    apiStub.listFolder.mockResolvedValue([])
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: null,
        encoding: 'windows1252' as const,
      },
    ]
    apiStub.fileRoots.mockResolvedValue(['/data'])
    await useFilesStore().restoreRoots()

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()

    expect(apiStub.saveStatementFile).toHaveBeenCalledWith({
      path: null,
      defaultName: 'Query 1.sql',
      defaultFolder: '/data',
      contents: 'SELECT 1',
      encoding: undefined,
    })
    // The new file is UTF-8, so the next save writes UTF-8.
    expect(tabs.tabs[0]).toMatchObject({
      filePath: '/data/daily.sql',
      title: 'daily.sql',
      dirty: false,
      encoding: 'utf8',
    })
  })

  it('holds the tab as it is when the user closed the save dialog', async () => {
    apiStub.saveStatementFile.mockResolvedValue(null)
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1.sql',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: null,
        encoding: 'utf8' as const,
      },
    ]
    await wrapper.setProps({ tab: tabs.tabs[0] })

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()

    // A title that already names a file keeps its one ending.
    expect(apiStub.saveStatementFile).toHaveBeenCalledWith(
      expect.objectContaining({ defaultName: 'Query 1.sql', defaultFolder: null }),
    )
    expect(tabs.tabs[0]?.filePath).toBeNull()
    expect(tabs.tabs[0]?.dirty).toBe(true)
  })

  it('reports a write that the disk refused', async () => {
    apiStub.saveStatementFile.mockRejectedValue({
      category: 'io',
      message: 'read only',
      detail: null,
    })
    const wrapper = await mountView('SELECT 1', '/data/report.sql')

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)

    // The button works again once the first write ends.
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/report.sql', encoding: 'utf8' })
    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()
    expect(apiStub.saveStatementFile).toHaveBeenCalledTimes(2)
  })

  it('starts one write at a time', async () => {
    let finish: () => void = () => {}
    apiStub.saveStatementFile.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = () => resolve(null)
        }),
    )
    const wrapper = await mountView('SELECT 1', '/data/report.sql')

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await wrapper.vm.saveToFile()
    expect(apiStub.saveStatementFile).toHaveBeenCalledTimes(1)

    finish()
    await settle()
  })

  it('moves the split between the editor and the results', async () => {
    const wrapper = await mountView()
    await wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [{ size: 70 }])
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'splitpanes' }).exists()).toBe(true)
  })

  it('ignores a split that reports no pane', async () => {
    const wrapper = await mountView()
    await wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [])
    expect(wrapper.findComponent({ name: 'splitpanes' }).exists()).toBe(true)
  })

  it('runs the statement that a command of the shell asks for', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    await mountView('SELECT 1;\nSELECT 2')
    tabActions('t1')?.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
      expect.anything(),
    )
  })

  it('runs the whole script that a command of the shell asks for', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    await mountView('SELECT 1;\nSELECT 2')
    tabActions('t1')?.runAll()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
      expect.anything(),
    )
  })

  it('lays out the statement when a command of the shell asks', async () => {
    const edits = editorWithText('select a from t')
    await mountView('select a from t')

    tabActions('t1')?.format()
    expect(edits).toHaveBeenCalled()
  })

  it('stops the statement that runs when a command of the shell asks', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockResolvedValue(undefined)
    await mountView()

    tabActions('t1')?.runAll()
    await settle()
    tabActions('t1')?.cancel()
    await settle()
    expect(apiStub.cancelQuery).toHaveBeenCalled()

    release(response)
    await settle()
  })

  it('writes the file when a command of the shell asks', async () => {
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/report.sql', encoding: 'utf8' })
    await mountView('SELECT 1', '/data/report.sql')

    tabActions('t1')?.save()
    await settle()
    expect(apiStub.saveStatementFile).toHaveBeenCalledWith(
      expect.objectContaining({ path: '/data/report.sql', contents: 'SELECT 1' }),
    )
  })

  it('forgets its actions when the tab goes away', async () => {
    const wrapper = await mountView()
    wrapper.unmount()
    expect(tabActions('t1')).toBeNull()
  })

  it('asks for the key list when the editor reports the key', async () => {
    const wrapper = await mountView()
    await wrapper.findComponent({ name: 'SqlEditor' }).vm.$emit('show-keys')
    expect(useUiStore().keyboardHelpOpen).toBe(true)
  })

  it('says so when a tab has no message yet', async () => {
    const wrapper = await mountView()
    expect(wrapper.find('[data-test="no-messages"]').exists()).toBe(true)
  })

  it('shows the messages the backend sent', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="messages-tab"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="query-message"]').text()).toBe('1 row returned.')
  })

  it('stops nothing when the tab holds no connection', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't3',
          title: 'Query 3',
          query: 'SELECT 1',
          connectionId: null,
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    const queries = useQueryStore()
    queries.stateFor('t3').running = true
    await wrapper.vm.$nextTick()
    await wrapper.find('[data-test="cancel-button"]').trigger('click')
    expect(apiStub.cancelQuery).not.toHaveBeenCalled()
  })
})

describe('QueryView details', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    // Most statements of these tests hold no parameter.
    apiStub.queryParameters.mockResolvedValue([])
  })

  it('falls back to the MS SQL Server dialect for a tab without a connection', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't9',
          title: 'Query 9',
          query: '',
          connectionId: null,
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'SqlEditor' }).props('dialect')).toBe('msSql')
  })

  it('falls back to the MS SQL Server dialect for a connection it does not know', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't10',
          title: 'Query 10',
          query: '',
          connectionId: 'ghost',
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8' as const,
        },
      },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'SqlEditor' }).props('dialect')).toBe('msSql')
  })

  it('runs the text of the tab when the editor gives nothing', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT 42')
    const view = wrapper.vm as unknown as { runStatement: (statement?: string) => void }
    view.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalled()
  })

  it('stays on the messages when a statement gives no result set', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({
        results: [],
        messages: [{ level: 'info' as const, text: '3 rows affected.', detail: null }],
        rowsAffected: 3,
        elapsedMs: 4,
      }),
    )
    const wrapper = await mountView('UPDATE t SET a = 1')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="query-message"]').text()).toBe('3 rows affected.')
  })
})

describe('QueryView edge paths', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    // Most statements of these tests hold no parameter.
    apiStub.queryParameters.mockResolvedValue([])
  })

  it('shows a failure that carries no cause', async () => {
    apiStub.executeQuery.mockRejectedValue({
      category: 'database',
      message: 'no such table',
      detail: null,
    })
    const wrapper = await mountView('SELECT 1')
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    const error = wrapper.find('[data-test="query-error"]')
    expect(error.text()).toContain('no such table')
    expect(error.find('.error-detail').exists()).toBe(false)
  })

  it('runs the text of the tab when no editor is in place', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView('SELECT 99')
    const view = wrapper.vm as unknown as { runStatement: (statement?: string) => void }
    wrapper.unmount()
    view.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 99' }),
      expect.anything(),
    )
  })

  it('records the encoding the backend used and says when it changed', async () => {
    apiStub.saveStatementFile.mockResolvedValue({ path: '/data/old.sql', encoding: 'utf8bom' })
    const wrapper = await mountView('SELECT 1', '/data/old.sql', 'windows1252')
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'old.sql',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        params: [],
        filePath: '/data/old.sql',
        encoding: 'windows1252',
      },
    ]
    await wrapper.setProps({ tab: tabs.tabs[0] })

    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()
    expect(apiStub.saveStatementFile).toHaveBeenCalledWith(
      expect.objectContaining({ path: '/data/old.sql', encoding: 'windows1252' }),
    )
    expect(tabs.tabs[0]?.encoding).toBe('utf8bom')
    expect(lastNotice()?.message).toContain("Windows-1252 can't store")

    // The same encoding back changes nothing on the tab.
    await wrapper.find('[data-test="save-file-button"]').trigger('click')
    await settle()
    expect(lastNotice()?.message).toBe('Saved old.sql.')
  })

  it('stays on the result that is open when a second statement runs', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(1)
  })

  it('lays out the statement when the button is pressed', async () => {
    const edits = editorWithText('select a from t')
    const wrapper = await mountView('select a from t')

    await wrapper.find('[data-test="format-button"]').trigger('click')
    await settle()

    expect(edits).toHaveBeenCalledWith('format', [
      expect.objectContaining({ text: 'SELECT\n  a\nFROM\n  t' }),
    ])
  })

  it('reports a statement that it cannot lay out', async () => {
    editorWithText('SELECT * FROM (')
    const wrapper = await mountView('SELECT * FROM (')

    await wrapper.find('[data-test="format-button"]').trigger('click')
    await settle()

    const ui = useUiStore()
    expect(ui.notices.some((notice) => notice.level === 'warning')).toBe(true)
  })

  it('keeps a result, names it with the time, and closes it again', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="pin-result"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Result 1 (1 row) at')

    // A second run keeps the result and adds the new one beside it.
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(2)

    await wrapper.find('[data-test="close-result"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(1)
  })

  it('closes a result that is not kept', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(1)

    await wrapper.find('[data-test="close-result"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(0)
  })

  it('keeps the place of the split that a drag leaves behind', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()

    wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [{ size: 62 }, { size: 38 }])
    await wrapper.vm.$nextTick()

    expect(layout.layout.editorSize).toBe(62)
  })

  it('puts the results panel away and shows a bar in its place', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()
    expect(wrapper.find('[data-test="results-bar"]').exists()).toBe(false)

    await wrapper.find('[data-test="collapse-results"]').trigger('click')
    await wrapper.vm.$nextTick()

    expect(layout.layout.resultsCollapsed).toBe(true)
    const bar = wrapper.find('[data-test="results-bar"]')
    expect(bar.exists()).toBe(true)
    // The messages stand open, so the bar names them.
    expect(bar.text()).toContain('Messages')
    // The editor takes the whole height, and the panel keeps its content.
    const panes = wrapper.findAllComponents({ name: 'pane' })
    expect(panes[0]?.props('size')).toBe(100)
    expect(panes[1]?.props('size')).toBe(0)
    expect(wrapper.find('[data-test="messages-tab"]').exists()).toBe(true)

    await wrapper.find('[data-test="expand-results"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(layout.layout.resultsCollapsed).toBe(false)
    expect(wrapper.find('[data-test="results-bar"]').exists()).toBe(false)
  })

  it('moves the results panel to the side of the editor and back', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()
    const panes = () => wrapper.findComponent({ name: 'splitpanes' })
    expect(panes().props('horizontal')).toBe(true)

    await wrapper.find('[data-test="move-results"]').trigger('click')
    await wrapper.vm.$nextTick()

    expect(layout.layout.resultsOrientation).toBe('beside')
    expect(panes().props('horizontal')).toBe(false)
    expect(wrapper.find('[data-test="move-results"]').attributes('aria-label')).toContain(
      'below the editor',
    )

    await wrapper.find('[data-test="move-results"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(layout.layout.resultsOrientation).toBe('below')
    expect(panes().props('horizontal')).toBe(true)
  })

  it('keeps one bar for the results panel in each place', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()
    layout.setResultsOrientation('beside')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="collapse-results"]').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="results-bar"]').exists()).toBe(true)
  })

  it('names the result that is open in the bar', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    await wrapper.find('[data-test="collapse-results"]').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="results-bar"]').text()).toContain('Result 1')
  })

  it('brings the results panel back when a statement runs', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    const layout = useLayoutStore()
    layout.setResultsCollapsed(true)
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(layout.layout.resultsCollapsed).toBe(false)
  })

  it('takes away the mark of the browser on each step of a drag', async () => {
    const wrapper = await mountView()
    const removeAllRanges = vi.fn()
    vi.spyOn(window, 'getSelection').mockReturnValue({
      removeAllRanges,
    } as unknown as Selection)

    wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [{ size: 55 }])
    await wrapper.vm.$nextTick()

    expect(removeAllRanges).toHaveBeenCalled()
    vi.mocked(window.getSelection).mockRestore()
  })

  it('holds a drag that reports no selection of the browser', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()
    vi.spyOn(window, 'getSelection').mockReturnValue(null)

    wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [{ size: 55 }])
    await wrapper.vm.$nextTick()

    expect(layout.layout.editorSize).toBe(55)
    vi.mocked(window.getSelection).mockRestore()
  })

  it('leaves the split alone when the drag reports no pane', async () => {
    const wrapper = await mountView()
    const layout = useLayoutStore()
    const before = layout.layout.editorSize

    wrapper.findComponent({ name: 'splitpanes' }).vm.$emit('resize', [])
    await wrapper.vm.$nextTick()

    expect(layout.layout.editorSize).toBe(before)
  })

  it('keeps a grid for the three results the user opened last', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response,
        results: Array.from({ length: 5 }, () => response.results[0]!),
      }),
    )
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    const tabs = wrapper.findComponent({ name: 'VTabs' })
    for (const pane of useQueryStore().stateFor('t1').panes) {
      await tabs.vm.$emit('update:model-value', pane.id)
      await wrapper.vm.$nextTick()
    }

    expect(wrapper.findAllComponents({ name: 'ResultsGrid' })).toHaveLength(3)
  })

  it('gives the place of a closed result to another result', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response,
        results: Array.from({ length: 4 }, () => response.results[0]!),
      }),
    )
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    const queries = useQueryStore()
    const tabs = wrapper.findComponent({ name: 'VTabs' })
    const panes = [...queries.stateFor('t1').panes]
    for (const pane of panes) {
      await tabs.vm.$emit('update:model-value', pane.id)
      await wrapper.vm.$nextTick()
    }
    expect(wrapper.findAllComponents({ name: 'ResultsGrid' })).toHaveLength(3)

    // The result that stands open closes, and the result that lost its grid
    // first takes the place that the closed result held.
    queries.closePane('t1', panes[3]!.id)
    await tabs.vm.$emit('update:model-value', panes[0]!.id)
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents({ name: 'ResultsGrid' })).toHaveLength(3)
  })

  it('hides the actions of a result while the messages stand open', async () => {
    const wrapper = await mountView()

    expect(wrapper.find('[data-test="pin-result"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="close-result"]').exists()).toBe(false)
  })

  it('moves between a result and the messages', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response))
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    const tabs = wrapper.findComponent({ name: 'VTabs' })
    await tabs.vm.$emit('update:model-value', 'messages')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="query-message"]').exists()).toBe(true)

    const paneId = useQueryStore().stateFor('t1').panes[0]!.id
    await tabs.vm.$emit('update:model-value', paneId)
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'ResultsGrid' }).exists()).toBe(true)
  })

  it('writes a result as a table of Markdown', async () => {
    apiStub.saveTextFile.mockResolvedValue('/tmp/out.md')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'markdown', exported())
    await settle()
    expect(apiStub.saveTextFile).toHaveBeenCalledWith(
      expect.objectContaining({ extension: 'md', contents: '| n |\n| --- |\n| 1 |' }),
    )
  })

  it('asks for the table before it writes INSERT statements', async () => {
    apiStub.saveTextFile.mockResolvedValue('/tmp/out.sql')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'insert', exported())
    await settle()
    expect(apiStub.saveTextFile).not.toHaveBeenCalled()

    const field = document.querySelector(
      '[data-test="insert-table-name"] input',
    ) as HTMLInputElement
    field.value = 'dbo.orders'
    field.dispatchEvent(new Event('input'))
    await settle()
    ;(document.querySelector('[data-test="insert-table-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.saveTextFile).toHaveBeenCalledWith(
      expect.objectContaining({
        extension: 'sql',
        contents: 'INSERT INTO [dbo].[orders] ([n]) VALUES (1);',
      }),
    )
  })

  it('names a placeholder table when the user gives no table name', async () => {
    apiStub.saveTextFile.mockResolvedValue('/tmp/out.sql')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'insert', exported())
    await settle()
    const field = document.querySelector(
      '[data-test="insert-table-name"] input',
    ) as HTMLInputElement
    field.value = '   '
    field.dispatchEvent(new Event('input'))
    await settle()
    ;(document.querySelector('[data-test="insert-table-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.saveTextFile).toHaveBeenCalledWith(
      expect.objectContaining({ contents: expect.stringContaining('the_table') }),
    )
  })

  it('asks the backend to write every row of a result that was cut', async () => {
    apiStub.exportQuery.mockResolvedValue({
      rows: 40000,
      truncated: false,
      path: '/tmp/all.csv',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'csv')
    await settle()

    expect(apiStub.exportQuery).toHaveBeenCalledWith(
      expect.objectContaining({
        connectionId: 'c1',
        query: 'SELECT 1',
        defaultName: expect.stringContaining('.csv'),
        format: 'csv',
        maxRows: 1000000,
        tabId: expect.any(String),
      }),
    )
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('writes every row of a kept result without a new run of the query', async () => {
    apiStub.exportKept.mockResolvedValue({
      rows: 40000,
      truncated: false,
      path: '/tmp/all.json',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    apiStub.executeQuery.mockImplementation(
      streamed({ ...response, kept: [{ set: 0, id: 'r1:0', origin: 'athena', keptAt: 5 }] }),
    )
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const grid = wrapper.findComponent({ name: 'ResultsGrid' })
    expect(grid.props('kept')).toEqual({ origin: 'athena', keptAt: 5 })
    await grid.vm.$emit('export-all', 'json')
    await settle()

    expect(apiStub.exportKept).toHaveBeenCalledWith({
      keptId: 'r1:0',
      requestId: expect.any(String),
      defaultName: expect.stringContaining('.json'),
      format: 'json',
      maxRows: 1000000,
    })
    expect(apiStub.exportQuery).not.toHaveBeenCalled()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  /** Runs a statement whose result paused at the row limit, and gives its grid. */
  async function pausedGrid() {
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response,
        kept: [{ set: 0, id: 'r1:0', origin: 'paused', keptAt: 0, pausedSecs: 600 }],
      }),
    )
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    const grid = wrapper.findComponent({ name: 'ResultsGrid' })
    expect(grid.props('pausedUntil')).toEqual(expect.any(Number))
    expect(grid.props('kept')).toMatchObject({ origin: 'paused' })
    return grid
  }

  it('releases a paused result when the user asks', async () => {
    const grid = await pausedGrid()
    await grid.vm.$emit('release')
    await settle()
    expect(apiStub.releaseKept).toHaveBeenCalledWith('r1:0')
    expect(grid.props('pausedUntil')).toBeUndefined()
    expect(grid.props('kept')).toBeNull()
  })

  it('ends the pause of a result after its export, and keeps it when the dialog closes', async () => {
    const grid = await pausedGrid()
    // The user closed the save dialog, so the read stays paused.
    apiStub.exportKept.mockResolvedValueOnce(null)
    await grid.vm.$emit('export-all', 'csv')
    await settle()
    expect(grid.props('pausedUntil')).toEqual(expect.any(Number))

    apiStub.exportKept.mockResolvedValueOnce({
      rows: 40000,
      truncated: false,
      path: '/tmp/all.csv',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    await grid.vm.$emit('export-all', 'csv')
    await settle()
    expect(grid.props('pausedUntil')).toBeUndefined()
    expect(grid.props('kept')).toBeNull()
    expect(apiStub.releaseKept).not.toHaveBeenCalled()
  })

  it('offers a new run when the kept result of an export is gone', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({ ...response, kept: [{ set: 0, id: 'r1:0', origin: 'athena', keptAt: 5 }] }),
    )
    apiStub.exportKept.mockRejectedValueOnce({
      category: 'keptGone',
      message: 'The saved result of this query is gone.',
      detail: null,
    })
    apiStub.exportQuery.mockResolvedValueOnce(null)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    const grid = wrapper.findComponent({ name: 'ResultsGrid' })
    await grid.vm.$emit('export-all', 'json')
    await settle()

    // The pane forgets the result, so its export menu says a new run follows.
    expect(grid.props('kept')).toBeNull()
    const notice = useUiStore().notices.find((entry) => entry.level === 'error')
    expect(notice?.message).toBe('The saved result of this query is gone.')
    expect(notice?.action?.label).toBe('Run again and export')
    expect(apiStub.exportQuery).not.toHaveBeenCalled()

    notice!.action!.run()
    await settle()
    expect(apiStub.exportQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1', format: 'json' }),
    )
    expect(apiStub.exportKept).toHaveBeenCalledTimes(1)
  })

  it('ends and releases the pause of a result whose export failed', async () => {
    const grid = await pausedGrid()
    apiStub.exportKept.mockRejectedValueOnce(new Error('The disk is full.'))
    await grid.vm.$emit('export-all', 'csv')
    await settle()
    expect(grid.props('pausedUntil')).toBeUndefined()
    expect(apiStub.releaseKept).toHaveBeenCalledWith('r1:0')
  })

  it('asks the backend for an Excel file of every row', async () => {
    apiStub.exportQuery.mockResolvedValue({
      rows: 40000,
      truncated: false,
      path: '/tmp/all.xlsx',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'xlsx')
    await settle()

    expect(apiStub.exportQuery).toHaveBeenCalledWith(
      expect.objectContaining({
        format: 'xlsx',
        defaultName: expect.stringContaining('.xlsx'),
      }),
    )
  })

  it('warns when the export limit stopped the read as well', async () => {
    apiStub.exportQuery.mockResolvedValue({
      rows: 1000000,
      truncated: true,
      path: '/tmp/all.json',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'json')
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
  })

  it('says when an Excel sheet was full', async () => {
    apiStub.exportQuery.mockResolvedValue({
      rows: 1048575,
      truncated: true,
      path: '/tmp/all.xlsx',
      sheetFull: true,
      cutCells: 0,
      warning: null,
    })
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'xlsx')
    await settle()
    const notice = lastNotice()
    expect(notice?.message).toContain('an Excel sheet has no room for more rows')
    expect(notice?.detail).toBe('Export to CSV to get every row.')
  })

  it('passes on a warning of the backend about the content of the file', async () => {
    const warning =
      '2 cells had more than 32,767 characters, the Excel limit for one cell, so their text was cut.'
    apiStub.exportQuery.mockResolvedValue({
      rows: 10,
      truncated: false,
      path: '/tmp/all.xlsx',
      sheetFull: false,
      cutCells: 2,
      warning,
    })
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'xlsx')
    await settle()

    const notices = useUiStore().notices
    expect(notices.some((notice) => notice.message === 'Exported 10 rows to /tmp/all.xlsx.')).toBe(
      true,
    )
    expect(lastNotice()?.level).toBe('warning')
    expect(lastNotice()?.message).toBe(warning)
  })

  it('shows a running export of all rows and stops it', async () => {
    let fail: (error: unknown) => void = () => {}
    apiStub.exportQuery.mockReturnValue(
      new Promise((_resolve, reject) => {
        fail = reject
      }),
    )
    let answer: (value: unknown) => void = () => {}
    apiStub.cancelQuery.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve
      }),
    )
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'csv')
    await settle()
    // The store of the tab keeps the export, so a view that mounts again
    // sees it.
    expect(useQueryStore().stateFor('t1').exporting?.connectionId).toBe('c1')
    expect(wrapper.find('[data-test="export-all-busy"]').exists()).toBe(true)
    expect(wrapper.findComponent({ name: 'ResultsGrid' }).props('exporting')).toBe(true)
    const pane = useQueryStore().stateFor('t1').panes[0]!
    await (
      wrapper.vm as unknown as { onExportAll: (p: ResultPane, f: 'json') => Promise<void> }
    ).onExportAll(pane, 'json')
    await settle()
    // A second request waits for the first one.
    expect(apiStub.exportQuery).toHaveBeenCalledTimes(1)
    expect(lastNotice()?.message).toBe('An export is already running in this tab.')

    await wrapper.find('[data-test="export-all-stop"]').trigger('click')
    await settle()
    const requestId = vi.mocked(apiStub.exportQuery).mock.calls[0]![0].requestId
    expect(apiStub.cancelQuery).toHaveBeenCalledWith('c1', requestId)
    expect(wrapper.find('[data-test="export-all-stop"]').text()).toBe('Stopping…')

    // A stop while the save dialog is open finds no request. The backend
    // answers without an error, and the button works again.
    answer(undefined)
    await settle()
    expect(wrapper.find('[data-test="export-all-stop"]').text()).toBe('Stop')
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(false)

    fail({ category: 'cancelled', message: 'stopped', detail: null })
    await settle()
    expect(wrapper.find('[data-test="export-all-busy"]').exists()).toBe(false)
    expect(lastNotice()?.message).toBe('Export stopped.')
  })

  it('reports a stop of an export that the backend refused', async () => {
    let finish: (value: unknown) => void = () => {}
    apiStub.exportQuery.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    apiStub.cancelQuery.mockRejectedValue({ category: 'internal', message: 'no', detail: null })
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'csv')
    await settle()
    await wrapper.find('[data-test="export-all-stop"]').trigger('click')
    await settle()
    expect(lastNotice()?.level).toBe('error')
    expect(wrapper.find('[data-test="export-all-stop"]').text()).toBe('Stop')

    finish(null)
    await settle()
    expect(wrapper.find('[data-test="export-all-busy"]').exists()).toBe(false)
  })

  it('writes no whole export for a plan', async () => {
    const wrapper = await mountView()
    await settle()
    const pane = { run: null } as unknown as ResultPane
    await (
      wrapper.vm as unknown as { onExportAll: (p: ResultPane, f: 'csv') => Promise<void> }
    ).onExportAll(pane, 'csv')
    expect(apiStub.exportQuery).not.toHaveBeenCalled()
    expect(
      useUiStore().notices.some((notice) => notice.message.includes('Run the statement first')),
    ).toBe(true)
  })

  it('writes no whole export when the user closes the save dialog', async () => {
    apiStub.exportQuery.mockResolvedValue(null)
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'csv')
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(false)
  })

  it('reports a whole export that failed', async () => {
    apiStub.exportQuery.mockRejectedValue({
      category: 'unsupported',
      message: 'only a read',
      detail: null,
    })
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export-all', 'csv')
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('writes every row of a kept result with its own statement and connection', async () => {
    apiStub.exportQuery.mockResolvedValue({
      rows: 1,
      truncated: false,
      path: '/tmp/all.csv',
      sheetFull: false,
      cutCells: 0,
      warning: null,
    })
    const wrapper = await mountedWithResult()
    await wrapper.find('[data-test="pin-result"]').trigger('click')
    await wrapper.vm.$nextTick()
    // The tab now names another statement and another connection.
    await wrapper.setProps({
      tab: { ...wrapper.props('tab'), query: 'SELECT 2', connectionId: 'c2' },
    })
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    const grids = wrapper.findAllComponents({ name: 'ResultsGrid' })
    const kept = grids.find(
      (grid) => grid.props('result') === useQueryStore().stateFor('t1').panes[0]!.result,
    )!
    await kept.vm.$emit('export-all', 'csv')
    await settle()

    expect(apiStub.exportQuery).toHaveBeenCalledWith(
      expect.objectContaining({ connectionId: 'c1', query: 'SELECT 1' }),
    )
  })

  it('closes the table dialog when the overlay reports it', async () => {
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'insert', exported())
    await settle()

    const dialog = wrapper
      .findAllComponents({ name: 'VDialog' })
      .find((item) => item.props('modelValue'))
    await dialog!.vm.$emit('update:modelValue', false)
    await settle()
    expect(dialog!.props('modelValue')).toBe(false)
  })

  it('writes no INSERT statements when the user closes the dialog', async () => {
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'insert', exported())
    await settle()
    ;(
      [...document.querySelectorAll('button')].find(
        (button) => button.textContent?.trim() === 'Cancel',
      ) as HTMLElement
    ).click()
    await settle()
    expect(apiStub.saveTextFile).not.toHaveBeenCalled()
  })

  it('writes a result as an Excel file', async () => {
    apiStub.saveBinaryFile.mockResolvedValue('/tmp/out.xlsx')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'xlsx', exported())
    await settle()
    const [request] = apiStub.saveBinaryFile.mock.calls[0] as [{ contents: string }]
    // A ZIP container starts with the two letters PK.
    expect(atob(request.contents).startsWith('PK')).toBe(true)
  })

  it('reports a result too wide for an Excel sheet and opens no save dialog', async () => {
    const wrapper = await mountedWithResult()
    const wide = {
      columns: Array.from({ length: 16385 }, (_, index) => ({
        name: `c${index}`,
        typeName: 'int',
      })),
      rows: [],
      truncated: false,
    }

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'xlsx', wide)
    await settle()
    expect(apiStub.saveBinaryFile).not.toHaveBeenCalled()
    const notice = useUiStore().notices.find((item) => item.level === 'error')
    expect(notice?.message).toContain('at most 16384 columns')
  })

  it('writes nothing when the user closes the save dialog', async () => {
    apiStub.saveTextFile.mockResolvedValue(null)
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(false)
  })
})

describe('QueryView run to file', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.queryParameters.mockResolvedValue([])
  })

  const summary = {
    rows: 40000,
    truncated: false,
    path: '/tmp/all.csv',
    sheetFull: false,
    cutCells: 0,
    warning: null,
    sets: [{ path: '/tmp/all.csv', sheet: null, rows: 40000, truncated: false, sheetFull: false }],
    skippedSets: 0,
  }

  /** Clicks a control of the dialog that asks where the results go. */
  async function clickRunFile(test: string, inner = '') {
    ;(document.querySelector(`[data-test="${test}"] ${inner}`.trim()) as HTMLElement).click()
    await settle()
  }

  /** A run to a file whose grid gets one row of a set that the limit cut. */
  function savedRun() {
    const stream = streamed({
      ...response,
      results: [{ columns: [{ name: 'n', typeName: 'int' }], rows: [[1]], truncated: true }],
    })
    apiStub.runToFile.mockImplementation(async (request: unknown, handlers: never) => {
      await stream(request, handlers)
      return summary
    })
  }

  it('runs once to the chosen file and shows the first rows with a note', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv' })
    savedRun()
    const wrapper = await mountView()

    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()

    expect(apiStub.chooseRunFile).toHaveBeenCalledWith({
      defaultName: expect.stringMatching(/^Query_1-.*\.csv$/),
    })
    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({
        connectionId: 'c1',
        query: 'SELECT 1',
        ticket: 'k1',
        eachSet: false,
      }),
      expect.anything(),
    )
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
    // A statement with one result asks nothing after the save dialog.
    expect(document.querySelector('[data-test="run-file-run"]')).toBeNull()
    expect(wrapper.find('[data-test="grid-saved-file"]').text()).toBe(
      'Showing the first 1 row. All 40,000 rows were saved to /tmp/all.csv.',
    )
    expect(lastNotice()?.level).toBe('success')
  })

  it('asks where the results of a script go and remembers the answer', async () => {
    localStorage.clear()
    apiStub.severalResultSets.mockResolvedValue(true)
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.xlsx', format: 'xlsx' })
    savedRun()
    apiStub.runToFile.mockResolvedValue({
      ...summary,
      path: '/tmp/all.xlsx',
      rows: 3,
      sets: [
        { path: '/tmp/all.xlsx', sheet: 'Result 1', rows: 1, truncated: false, sheetFull: false },
        { path: '/tmp/all.xlsx', sheet: 'Result 2', rows: 2, truncated: false, sheetFull: false },
      ],
    })
    const wrapper = await mountView('SELECT 1; SELECT 2')

    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.severalResultSets).toHaveBeenCalledWith('SELECT 1; SELECT 2', 'msSql')
    expect(document.body.textContent).toContain('One sheet per result set')
    expect(apiStub.runToFile).not.toHaveBeenCalled()
    await clickRunFile('run-file-each', 'input')
    await clickRunFile('run-file-run')

    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({ ticket: 'k1', eachSet: true }),
      expect.anything(),
    )
    expect(lastNotice()?.message).toBe('Saved 3 rows to 2 sheets in /tmp/all.xlsx.')
    expect(localStorage.getItem('sql-explorer.runFileSets')).toBe('each')
  })

  it('runs nothing when the user cancels the question about the results', async () => {
    apiStub.severalResultSets.mockResolvedValue(true)
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv', format: 'csv' })
    const wrapper = await mountView('SELECT 1; SELECT 2')
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    await clickRunFile('run-file-cancel')
    expect(apiStub.runToFile).not.toHaveBeenCalled()
  })

  it('says when results after the first went to the grid alone', async () => {
    apiStub.severalResultSets.mockRejectedValue(new Error('no lexer'))
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv', format: 'csv' })
    apiStub.runToFile.mockResolvedValue({ ...summary, skippedSets: 2 })
    const wrapper = await mountView('EXEC report')
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    // A failed check counts as one result, so the run starts at once.
    expect(apiStub.runToFile).toHaveBeenCalled()
    expect(lastNotice()?.message).toBe('Only the first result went to the file.')
  })

  it('offers a CSV file when the row limit is past the room of an Excel sheet', async () => {
    apiStub.chooseRunFile
      .mockResolvedValueOnce({ ticket: 'k1', path: '/tmp/big.xlsx', format: 'xlsx' })
      .mockResolvedValueOnce({ ticket: 'k2', path: '/tmp/big.csv', format: 'csv' })
    apiStub.runToFile.mockResolvedValue(summary)
    const wrapper = await mountView()
    useSettingsStore().update({ exportRowLimit: 2_000_000 })
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(document.querySelector('[data-test="run-file-excel-limit"]')).not.toBeNull()
    await clickRunFile('run-file-csv')

    expect(apiStub.chooseRunFile).toHaveBeenLastCalledWith({ defaultName: 'big.csv' })
    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({ ticket: 'k2', eachSet: false }),
      expect.anything(),
    )
  })

  it('says when a full Excel sheet left rows out of the file', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/a.xlsx', format: 'xlsx' })
    apiStub.runToFile.mockResolvedValue({ ...summary, rows: 1048575, sheetFull: true })
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(lastNotice()?.message).toBe(
      "Saved 1,048,575 rows. An Excel sheet has no room for more, so the rest weren't saved.",
    )
    expect(lastNotice()?.detail).toBe('Run to file as CSV to save every row.')
  })

  it('runs nothing when the user closes the save dialog', async () => {
    apiStub.chooseRunFile.mockResolvedValue(null)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.runToFile).not.toHaveBeenCalled()
  })

  it('reports a failure to open the save dialog', async () => {
    apiStub.chooseRunFile.mockRejectedValue({ category: 'internal', message: 'no', detail: null })
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.runToFile).not.toHaveBeenCalled()
    expect(lastNotice()?.level).toBe('error')
  })

  it('reports nothing more for a run that failed', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv' })
    apiStub.runToFile.mockRejectedValue({ category: 'query', message: 'bad', detail: null })
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(false)
  })

  /** The notice that offers to try the run to a file again. */
  function retryNotice() {
    return useUiStore().notices.find((notice) => notice.action?.label === 'Try again')
  }

  it('offers to try again with the same file when the run did not start', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv' })
    apiStub.runToFile.mockRejectedValueOnce({
      category: 'connection',
      message: 'refused',
      detail: null,
    })
    apiStub.runFileReady.mockResolvedValue(true)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.runFileReady).toHaveBeenCalledWith('k1')
    const notice = retryNotice()
    expect(notice?.message).toBe("Run to file didn't start, so nothing was saved.")
    expect(notice?.timeout).toBe(-1)

    savedRun()
    notice?.action?.run()
    await settle()
    expect(apiStub.chooseRunFile).toHaveBeenCalledOnce()
    expect(apiStub.runToFile).toHaveBeenLastCalledWith(
      expect.objectContaining({ ticket: 'k1', eachSet: false }),
      expect.anything(),
    )
    expect(lastNotice()?.level).toBe('success')
  })

  it('offers no second try for a stopped run or a used ticket', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv' })
    apiStub.runToFile.mockRejectedValue({ category: 'cancelled', message: 'stop', detail: null })
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.runFileReady).not.toHaveBeenCalled()

    apiStub.runToFile.mockRejectedValue({ category: 'query', message: 'bad', detail: null })
    apiStub.runFileReady.mockRejectedValue(new Error('gone'))
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(apiStub.runFileReady).toHaveBeenCalled()
    expect(retryNotice()).toBeUndefined()
  })

  it('checks the tab again before a second try and clears the offer on close', async () => {
    apiStub.chooseRunFile.mockResolvedValue({ ticket: 'k1', path: '/tmp/all.csv' })
    apiStub.runToFile.mockRejectedValue({ category: 'connection', message: 'no', detail: null })
    apiStub.runFileReady.mockResolvedValue(true)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    useQueryStore().stateFor('t1').running = true
    retryNotice()?.action?.run()
    await settle()
    expect(apiStub.runToFile).toHaveBeenCalledOnce()
    expect(lastNotice()?.message).toBe('A statement is already running in this tab.')

    useQueryStore().stateFor('t1').running = false
    await settle()
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(retryNotice()).toBeDefined()
    wrapper.unmount()
    expect(retryNotice()).toBeUndefined()
  })

  it('asks for the values of the parameters before the file', async () => {
    apiStub.queryParameters.mockResolvedValue(['id'])
    const wrapper = await mountView('SELECT :id')
    await wrapper.find('[data-test="run-to-file-button"]').trigger('click')
    await settle()
    expect(document.querySelector('[data-test="parameters-confirm"]')).not.toBeNull()
    expect(apiStub.chooseRunFile).not.toHaveBeenCalled()
  })

  it('opens no dialog for a run that cannot start', async () => {
    const wrapper = await mountView('   ')
    const view = wrapper.vm as unknown as { runToFile: () => void }
    view.runToFile()
    await settle()
    expect(lastNotice()?.message).toBe('There is nothing to run.')

    const running = await mountView()
    useQueryStore().stateFor('t1').running = true
    ;(running.vm as unknown as { runToFile: () => void }).runToFile()
    await settle()
    expect(lastNotice()?.message).toBe('A statement is already running in this tab.')
    useQueryStore().stateFor('t1').running = false

    const closed = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't2',
          title: 'Query 2',
          query: 'SELECT 1',
          connectionId: null,
          dirty: false,
          params: [],
          filePath: null,
          encoding: 'utf8',
        },
      },
    })
    ;(closed.vm as unknown as { runToFile: () => void }).runToFile()
    await settle()
    expect(lastNotice()?.message).toBe('Choose a connection to run this statement.')
    expect(apiStub.chooseRunFile).not.toHaveBeenCalled()
  })

  it('reaches the run to file through the actions of the tab', async () => {
    apiStub.chooseRunFile.mockResolvedValue(null)
    await mountView()
    tabActions('t1')?.runToFile()
    await settle()
    expect(apiStub.chooseRunFile).toHaveBeenCalled()
  })
})
