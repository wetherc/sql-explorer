import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const saveDialog = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: (...args: unknown[]) => saveDialog(...args),
  open: vi.fn(),
}))

const { monaco } = await import('@/plugins/monaco')
const { tabActions } = await import('@/lib/commands')
const QueryView = (await import('@/components/QueryView.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useQueryStore } = await import('@/stores/query')
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
  messages: ['1 row returned.'],
  rowsAffected: null,
  elapsedMs: 8,
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
  } as unknown as ReturnType<typeof monaco.editor.create>)
  return executeEdits
}

/** Mounts the view and runs one statement, so a grid is on show. */
async function mountedWithResult() {
  apiStub.executeQuery.mockResolvedValue(response)
  const wrapper = await mountView()
  await wrapper.find('[data-test="run-button"]').trigger('click')
  await settle()
  await wrapper.vm.$nextTick()
  return wrapper
}

async function mountView(query = 'SELECT 1') {
  const wrapper = mountWithPlugins(QueryView, {
    props: {
      tab: {
        id: 't1',
        title: 'Query 1',
        query,
        connectionId: 'c1',
        dirty: false,
        savedQueryId: null,
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
    saveDialog.mockReset()
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
  })

  it('sends the statement to the backend when Run is pressed', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView()

    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ connectionId: 'c1', query: 'SELECT 1' }),
    )
  })

  it('sends the whole script when Run all is pressed', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView('SELECT 1;\nSELECT 2')

    await wrapper.find('[data-test="run-all-button"]').trigger('click')
    await settle()

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
    )
  })

  it('shows the result after a statement runs', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="result-tab"]')).toHaveLength(1)
    expect(wrapper.text()).toContain('Result 1 (1 row)')
  })

  it('shows the reason a statement failed', async () => {
    apiStub.executeQuery.mockRejectedValue({
      kind: 'database',
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
          savedQueryId: null,
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
        savedQueryId: null,
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
        savedQueryId: null,
      },
    ]
    await wrapper.findComponent({ name: 'VSelect' }).vm.$emit('update:modelValue', 'c2')
    expect(tabs.tabs[0]?.connectionId).toBe('c2')
  })

  it('writes a result to the file the user chose', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    apiStub.writeTextFile.mockResolvedValue(undefined)
    saveDialog.mockResolvedValue('/tmp/out.csv')

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()

    expect(apiStub.writeTextFile).toHaveBeenCalledWith('/tmp/out.csv', 'n\n1')
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('writes a result as JSON', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    apiStub.writeTextFile.mockResolvedValue(undefined)
    saveDialog.mockResolvedValue('/tmp/out.json')

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'json', exported())
    await settle()
    expect(apiStub.writeTextFile).toHaveBeenCalledWith(
      '/tmp/out.json',
      '[\n  {\n    "n": 1\n  }\n]',
    )
  })

  it('writes nothing when the user closed the file dialog', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    saveDialog.mockResolvedValue(null)

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(apiStub.writeTextFile).not.toHaveBeenCalled()
  })

  it('reports a failure to write a file', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    saveDialog.mockRejectedValue({ kind: 'io', message: 'read only', detail: null })

    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
  })

  it('notes that the rows reached the clipboard', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView()
    await wrapper.find('[data-test="run-button"]').trigger('click')
    await settle()
    await wrapper.vm.$nextTick()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('copied', 'n\n1')
    expect(useUiStore().notices.some((notice) => notice.level === 'success')).toBe(true)
  })

  it('saves the statement under a name', async () => {
    apiStub.saveQuery.mockResolvedValue(undefined)
    const wrapper = await mountView()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 't1',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: 'c1',
        dirty: true,
        savedQueryId: null,
      },
    ]

    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()

    const name = document.querySelector('[data-test="save-query-name"] input') as HTMLInputElement
    name.value = 'Daily count'
    name.dispatchEvent(new Event('input'))
    await settle()

    const confirm = document.querySelector('[data-test="save-query-confirm"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.saveQuery).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'Daily count', query: 'SELECT 1' }),
    )
    expect(tabs.tabs[0]?.title).toBe('Daily count')
    expect(tabs.tabs[0]?.dirty).toBe(false)
  })

  it('keeps the dialog open when the statement could not be saved', async () => {
    apiStub.saveQuery.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const wrapper = await mountView()

    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()
    const confirm = document.querySelector('[data-test="save-query-confirm"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(useUiStore().notices.some((notice) => notice.level === 'error')).toBe(true)
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
    apiStub.executeQuery.mockResolvedValue(response)
    await mountView('SELECT 1;\nSELECT 2')
    tabActions('t1')?.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
    )
  })

  it('runs the whole script that a command of the shell asks for', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    await mountView('SELECT 1;\nSELECT 2')
    tabActions('t1')?.runAll()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
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
    apiStub.executeQuery.mockResolvedValue(response)
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
          savedQueryId: null,
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
    saveDialog.mockReset()
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
  })

  it('closes the save dialog without saving', async () => {
    const wrapper = await mountView()
    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()

    const cancel = [...document.querySelectorAll('.v-card-actions .v-btn')].find((button) =>
      button.textContent?.includes('Cancel'),
    )
    cancel?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.saveQuery).not.toHaveBeenCalled()
  })

  it('keeps the folder the user typed for a saved statement', async () => {
    apiStub.saveQuery.mockResolvedValue(undefined)
    const wrapper = await mountView()
    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()

    const folder = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.props('label') === 'Folder')
    await folder?.vm.$emit('update:modelValue', 'Reports')

    const confirm = document.querySelector('[data-test="save-query-confirm"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.saveQuery).toHaveBeenCalledWith(expect.objectContaining({ folder: 'Reports' }))
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
          savedQueryId: null,
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
          savedQueryId: null,
        },
      },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'SqlEditor' }).props('dialect')).toBe('msSql')
  })

  it('runs the text of the tab when the editor gives nothing', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView('SELECT 42')
    const view = wrapper.vm as unknown as { runStatement: (statement?: string) => void }
    view.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalled()
  })

  it('stays on the messages when a statement gives no result set', async () => {
    apiStub.executeQuery.mockResolvedValue({
      results: [],
      messages: ['3 rows affected.'],
      rowsAffected: 3,
      elapsedMs: 4,
    })
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
    saveDialog.mockReset()
    // Gives back the editor stub of the test setup, so that a test which
    // puts its own editor in place does not reach the next one.
    vi.mocked(monaco.editor.create).mockReset()
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
    apiStub.saveQuery.mockResolvedValue(undefined)
  })

  it('shows a failure that carries no cause', async () => {
    apiStub.executeQuery.mockRejectedValue({
      kind: 'database',
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
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView('SELECT 99')
    const view = wrapper.vm as unknown as { runStatement: (statement?: string) => void }
    wrapper.unmount()
    view.runStatement()
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 99' }),
    )
  })

  it('saves again under the identifier the tab came from', async () => {
    const wrapper = mountWithPlugins(QueryView, {
      props: {
        tab: {
          id: 't11',
          title: 'Daily',
          query: 'SELECT 1',
          connectionId: 'c1',
          dirty: true,
          savedQueryId: 'q7',
        },
      },
    })
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()
    const confirm = document.querySelector('[data-test="save-query-confirm"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.saveQuery).toHaveBeenCalledWith(expect.objectContaining({ id: 'q7' }))
  })

  it('stays on the result that is open when a second statement runs', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
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
    apiStub.executeQuery.mockResolvedValue(response)
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

  it('moves between a result and the messages', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
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
    apiStub.writeTextFile.mockResolvedValue(undefined)
    saveDialog.mockResolvedValue('/tmp/out.md')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'markdown', exported())
    await settle()
    expect(apiStub.writeTextFile).toHaveBeenCalledWith('/tmp/out.md', '| n |\n| --- |\n| 1 |')
  })

  it('asks for the table before it writes INSERT statements', async () => {
    apiStub.writeTextFile.mockResolvedValue(undefined)
    saveDialog.mockResolvedValue('/tmp/out.sql')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'insert', exported())
    await settle()
    expect(apiStub.writeTextFile).not.toHaveBeenCalled()

    const field = document.querySelector(
      '[data-test="insert-table-name"] input',
    ) as HTMLInputElement
    field.value = 'dbo.orders'
    field.dispatchEvent(new Event('input'))
    await settle()
    ;(document.querySelector('[data-test="insert-table-confirm"]') as HTMLElement).click()
    await settle()

    expect(apiStub.writeTextFile).toHaveBeenCalledWith(
      '/tmp/out.sql',
      'INSERT INTO [dbo].[orders] ([n]) VALUES (1);',
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
    expect(saveDialog).not.toHaveBeenCalled()
  })

  it('writes a result as an Excel file', async () => {
    apiStub.writeBinaryFile.mockResolvedValue(undefined)
    saveDialog.mockResolvedValue('/tmp/out.xlsx')
    const wrapper = await mountedWithResult()

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'xlsx', exported())
    await settle()
    const [path, base64] = apiStub.writeBinaryFile.mock.calls[0] as [string, string]
    expect(path).toBe('/tmp/out.xlsx')
    // A ZIP container starts with the two letters PK.
    expect(atob(base64).startsWith('PK')).toBe(true)
  })

  it('writes nothing when the user closes the save dialog', async () => {
    saveDialog.mockResolvedValue(null)
    const wrapper = await mountedWithResult()
    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv', exported())
    await settle()
    expect(apiStub.writeTextFile).not.toHaveBeenCalled()
  })

  it('closes the save dialog when the overlay reports it', async () => {
    const wrapper = await mountView()
    await wrapper.find('[data-test="save-query-button"]').trigger('click')
    await settle()

    const dialog = wrapper
      .findAllComponents({ name: 'VDialog' })
      .find((item) => item.props('modelValue'))!
    await dialog.vm.$emit('update:modelValue', false)
    await settle()
    expect(dialog.props('modelValue')).toBe(false)
  })
})
