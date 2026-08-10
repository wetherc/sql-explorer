import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const saveDialog = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: (...args: unknown[]) => saveDialog(...args),
  open: vi.fn(),
}))

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

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv')
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

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'json')
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

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv')
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

    await wrapper.findComponent({ name: 'ResultsGrid' }).vm.$emit('export', 'csv')
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

  it('runs the statement the editor asks for', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView('SELECT 1;\nSELECT 2')
    await wrapper.findComponent({ name: 'SqlEditor' }).vm.$emit('execute', 'SELECT 2')
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 2' }),
    )
  })

  it('runs the whole script when the editor asks for it', async () => {
    apiStub.executeQuery.mockResolvedValue(response)
    const wrapper = await mountView('SELECT 1;\nSELECT 2')
    await wrapper.findComponent({ name: 'SqlEditor' }).vm.$emit('execute-all')
    await settle()
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1;\nSELECT 2' }),
    )
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
