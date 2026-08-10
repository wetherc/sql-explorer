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
