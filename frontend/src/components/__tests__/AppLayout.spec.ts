import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))

const AppLayout = (await import('@/layouts/AppLayout.vue')).default
const App = (await import('@/App.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useExplorerStore } = await import('@/stores/explorer')
const { useSettingsStore } = await import('@/stores/settings')
const { useTabsStore } = await import('@/stores/tabs')
const { useUiStore } = await import('@/stores/ui')
const { forgetTabActions, registerTabActions } = await import('@/lib/commands')
const { ConnectionHealth } = await import('@/types/api')

describe('AppLayout', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.supportedEngines.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
    apiStub.getWorkspace.mockResolvedValue({ tabs: [], activeTabId: null })
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    apiStub.onConnectionStatus.mockResolvedValue(() => {})
  })

  it('reads everything it needs when it opens', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    expect(apiStub.supportedEngines).toHaveBeenCalled()
    expect(apiStub.getConnections).toHaveBeenCalled()
    expect(apiStub.getHistory).toHaveBeenCalled()
    expect(apiStub.getWorkspace).toHaveBeenCalled()
    expect(apiStub.onConnectionStatus).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('adds a root for each connection that was already open', async () => {
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    expect(useExplorerStore().roots).toHaveLength(1)
    wrapper.unmount()
  })

  it('records the state a connection reports', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    const handler = apiStub.onConnectionStatus.mock.calls[0]?.[0] as (event: unknown) => void
    handler({ connectionId: 'c1', health: ConnectionHealth.Reconnecting, message: null })
    expect(useConnectionsStore().health.c1).toBe(ConnectionHealth.Reconnecting)
    wrapper.unmount()
  })

  it('moves between the three panels', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    await wrapper.find('[data-test="rail-explorer"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'DbExplorer' }).isVisible()).toBe(true)

    await wrapper.find('[data-test="rail-history"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'HistoryPanel' }).isVisible()).toBe(true)

    await wrapper.find('[data-test="rail-connections"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'ConnectionManager' }).isVisible()).toBe(true)
    wrapper.unmount()
  })

  it('shows the explorer once a connection opens', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    await wrapper.findComponent({ name: 'ConnectionManager' }).vm.$emit('connected', 'c1')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'DbExplorer' }).isVisible()).toBe(true)
    wrapper.unmount()
  })

  it('shows the connections when a panel asks for them', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    await wrapper.find('[data-test="rail-explorer"]').trigger('click')
    await wrapper.findComponent({ name: 'DbExplorer' }).vm.$emit('open-connections')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'ConnectionManager' }).isVisible()).toBe(true)

    await wrapper.find('[data-test="rail-explorer"]').trigger('click')
    await wrapper.findComponent({ name: 'QueryTabs' }).vm.$emit('open-connections')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent({ name: 'ConnectionManager' }).isVisible()).toBe(true)
    wrapper.unmount()
  })

  it('opens a tab from the application bar', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    await wrapper.find('[data-test="app-new-query"]').trigger('click')
    expect(useTabsStore().tabs).toHaveLength(1)
    wrapper.unmount()
  })

  it('moves between the two themes', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    const settings = useSettingsStore()
    expect(settings.isDark).toBe(true)
    await wrapper.find('[data-test="theme-toggle"]').trigger('click')
    await wrapper.vm.$nextTick()
    expect(settings.settings.theme).toBe('sqlExplorerLight')
    wrapper.unmount()
  })

  it('opens the settings and changes them', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    await wrapper.find('[data-test="open-settings"]').trigger('click')
    await settle()

    const settings = useSettingsStore()
    const rows = document.querySelector('[data-test="setting-max-rows"] input') as HTMLInputElement
    rows.value = '250'
    rows.dispatchEvent(new Event('input'))
    await settle()
    expect(settings.settings.maxRows).toBe(250)

    const slider = wrapper.findComponent({ name: 'VSlider' })
    await slider.vm.$emit('update:modelValue', 17)
    expect(settings.settings.fontSize).toBe(17)

    const switches = wrapper.findAllComponents({ name: 'VSwitch' })
    await switches[0]!.vm.$emit('update:modelValue', true)
    await switches[1]!.vm.$emit('update:modelValue', false)
    await switches[2]!.vm.$emit('update:modelValue', false)
    expect(settings.settings.wordWrap).toBe(true)
    expect(settings.settings.showLineNumbers).toBe(false)
    expect(settings.settings.autoRunPreview).toBe(false)
    wrapper.unmount()
  })

  it('writes the open tabs back when they change', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    apiStub.saveWorkspace.mockClear()

    useTabsStore().add({ query: 'SELECT 1' })
    await settle()
    expect(apiStub.saveWorkspace).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('stops listening when it goes away', async () => {
    const unlisten = vi.fn()
    apiStub.onConnectionStatus.mockResolvedValue(unlisten)
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    wrapper.unmount()
    expect(unlisten).toHaveBeenCalled()
  })

  it('draws the layout from the root of the application', async () => {
    const wrapper = mountWithPlugins(App)
    await settle()
    expect(wrapper.findComponent({ name: 'AppLayout' }).exists()).toBe(true)
    wrapper.unmount()
  })
})

describe('AppLayout settings dialog', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.supportedEngines.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
    apiStub.getWorkspace.mockResolvedValue({ tabs: [], activeTabId: null })
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    apiStub.onConnectionStatus.mockResolvedValue(() => {})
  })

  it('closes the settings from its own button', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    await wrapper.find('[data-test="open-settings"]').trigger('click')
    await settle()

    const openDialogs = () =>
      wrapper.findAllComponents({ name: 'VDialog' }).filter((item) => item.props('modelValue'))
    expect(openDialogs()).toHaveLength(1)

    const close = [...document.querySelectorAll('.v-card-actions .v-btn')].find((button) =>
      button.textContent?.includes('Close'),
    )
    close?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(openDialogs()).toHaveLength(0)
    wrapper.unmount()
  })
})

describe('AppLayout dialog state', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.supportedEngines.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
    apiStub.getWorkspace.mockResolvedValue({ tabs: [], activeTabId: null })
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    apiStub.onConnectionStatus.mockResolvedValue(() => {})
  })

  it('writes the limit of the kept results', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    await wrapper.find('[data-test="open-settings"]').trigger('click')
    await settle()

    const field = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'setting-max-pinned')
    await field!.vm.$emit('update:modelValue', '3')
    expect(useSettingsStore().settings.maxPinnedResults).toBe(3)

    const limit = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'setting-export-limit')
    await limit!.vm.$emit('update:modelValue', '5000')
    expect(useSettingsStore().settings.exportRowLimit).toBe(5000)

    const price = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'setting-athena-price')
    await price!.vm.$emit('update:modelValue', '6.5')
    expect(useSettingsStore().settings.athenaPricePerTerabyte).toBe(6.5)

    const warn = wrapper
      .findAllComponents({ name: 'VTextField' })
      .find((item) => item.attributes('data-test') === 'setting-athena-warning')
    await warn!.vm.$emit('update:modelValue', '25')
    expect(useSettingsStore().settings.athenaScanWarningGb).toBe(25)
    wrapper.unmount()
  })

  it('closes the settings when the overlay reports it', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    await wrapper.find('[data-test="open-settings"]').trigger('click')
    await settle()

    const open = () =>
      wrapper.findAllComponents({ name: 'VDialog' }).filter((item) => item.props('modelValue'))
    expect(open()).toHaveLength(1)
    await open()[0]!.vm.$emit('update:modelValue', false)
    // The dialog moves the focus back on the next tick, so it must still
    // be mounted when that runs.
    await settle()
    expect(open()).toHaveLength(0)
    wrapper.unmount()
  })
})

describe('AppLayout keys', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.supportedEngines.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
    apiStub.getWorkspace.mockResolvedValue({ tabs: [], activeTabId: null })
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    apiStub.onConnectionStatus.mockResolvedValue(() => {})
    forgetTabActions('key-tab')
  })

  /** Sends one key to the window, as the host does. */
  function press(code: string, parts: { shift?: boolean; alt?: boolean; mod?: boolean } = {}) {
    const event = new KeyboardEvent('keydown', {
      code,
      ctrlKey: parts.mod ?? true,
      shiftKey: parts.shift ?? false,
      altKey: parts.alt ?? false,
      cancelable: true,
    })
    window.dispatchEvent(event)
    return event
  }

  it('opens a tab and closes it again', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    const tabs = useTabsStore()

    press('KeyT')
    expect(tabs.tabs).toHaveLength(1)

    press('KeyW')
    expect(tabs.tabs).toHaveLength(0)
    wrapper.unmount()
  })

  it('moves between the three panels', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()

    press('Digit2')
    await settle()
    expect(wrapper.find('[data-test="rail-explorer"]').classes()).toContain('v-list-item--active')

    press('Digit3')
    await settle()
    expect(wrapper.find('[data-test="rail-history"]').classes()).toContain('v-list-item--active')

    press('Digit1')
    await settle()
    expect(wrapper.find('[data-test="rail-connections"]').classes()).toContain(
      'v-list-item--active',
    )
    wrapper.unmount()
  })

  it('opens the settings, the palette and the key list', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    const ui = useUiStore()

    press('Comma')
    await settle()
    expect(document.querySelector('[data-test="setting-max-rows"]')).not.toBeNull()

    // A dialog holds the keys, so the next two are opened through the store.
    ui.setPaletteOpen(true)
    await settle()
    expect(document.querySelector('[data-test="palette-filter"]')).not.toBeNull()
    ui.setPaletteOpen(false)
    await settle()

    ui.setKeyboardHelpOpen(true)
    await settle()
    expect(document.querySelectorAll('[data-test="key-list-row"]').length).toBeGreaterThan(5)

    const keyList = document
      .querySelector('[data-test="key-list-row"]')
      ?.closest('.v-card') as HTMLElement
    const close = [...keyList.querySelectorAll('button')].find(
      (button) => button.textContent?.trim() === 'Close',
    ) as HTMLElement
    close.click()
    await settle()
    expect(ui.keyboardHelpOpen).toBe(false)
    wrapper.unmount()
  })

  it('opens the palette and the key list with their keys', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    const ui = useUiStore()

    press('KeyP', { shift: true })
    expect(ui.paletteOpen).toBe(true)
    ui.setPaletteOpen(false)
    await settle()

    press('F1', { mod: false })
    expect(ui.keyboardHelpOpen).toBe(true)
    wrapper.unmount()
  })

  it('reaches the view of the tab that is open', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    const tabs = useTabsStore()
    tabs.tabs = [
      {
        id: 'key-tab',
        title: 'Query 1',
        query: 'SELECT 1',
        connectionId: null,
        dirty: false,
        savedQueryId: null,
      },
    ]
    tabs.activeTabId = 'key-tab'
    const actions = {
      runStatement: vi.fn(),
      runAll: vi.fn(),
      cancel: vi.fn(),
      format: vi.fn(),
    }
    registerTabActions('key-tab', actions)

    press('Enter')
    press('Enter', { shift: true })
    press('KeyC', { shift: true })
    press('KeyF', { mod: false, shift: true, alt: true })

    expect(actions.runStatement).toHaveBeenCalled()
    expect(actions.runAll).toHaveBeenCalled()
    expect(actions.cancel).toHaveBeenCalled()
    expect(actions.format).toHaveBeenCalled()
    wrapper.unmount()
  })

  it('leaves a key alone when no command holds it', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    const event = press('KeyZ')
    expect(event.defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it('leaves a command alone while it cannot run', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    // No tab is open, so the commands of a query cannot run.
    const event = press('Enter')
    expect(event.defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it('lets a dialog keep the keys while it stands open', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    await wrapper.find('[data-test="open-settings"]').trigger('click')
    await settle()

    const tabs = useTabsStore()
    press('KeyT')
    expect(tabs.tabs).toHaveLength(0)
    wrapper.unmount()
  })

  it('stops listening once it is gone', async () => {
    const wrapper = mountWithPlugins(AppLayout)
    await settle()
    wrapper.unmount()
    await settle()
    const tabs = useTabsStore()
    press('KeyT')
    expect(tabs.tabs).toHaveLength(0)
  })
})
