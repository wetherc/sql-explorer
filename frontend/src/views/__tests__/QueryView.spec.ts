import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import QueryView from '../QueryView.vue'
import { useQueryStore } from '@/stores/query'
import { useConnectionStore } from '@/stores/connection'

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('QueryView.vue', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders the initial layout', () => {
    const wrapper = mount(QueryView)
    expect(wrapper.find('h1').text()).toBe('SQL Explorer')
    expect(wrapper.find('textarea').exists()).toBe(true)
    expect(wrapper.find('button[type="submit"]').text()).toBe('Execute')
  })

  it('calls executeQuery action on button click', async () => {
    const wrapper = mount(QueryView)
    const queryStore = useQueryStore()
    const executeSpy = vi.spyOn(queryStore, 'executeQuery')
    const query = 'SELECT * FROM users'

    await wrapper.find('textarea').setValue(query)
    await wrapper.find('form').trigger('submit.prevent')

    expect(executeSpy).toHaveBeenCalledTimes(1)
    expect(executeSpy).toHaveBeenCalledWith(query)
  })

  it('displays results in a table', async () => {
    const wrapper = mount(QueryView)
    const queryStore = useQueryStore()
    const results = [
      { id: 1, name: 'Alice' },
      { id: 2, name: 'Bob' },
    ]

    // @ts-ignore
    queryStore.results = results
    await wrapper.vm.$nextTick()

    const table = wrapper.find('table')
    expect(table.exists()).toBe(true)

    const headers = table.findAll('th')
    expect(headers.length).toBe(2)
    expect(headers[0].text()).toBe('id')
    expect(headers[1].text()).toBe('name')

    const rows = table.findAll('tbody tr')
    expect(rows.length).toBe(2)
    expect(rows[0].text()).toContain('1')
    expect(rows[0].text()).toContain('Alice')
    expect(rows[1].text()).toContain('2')
    expect(rows[1].text()).toContain('Bob')
  })

  it('displays an error message', async () => {
    const wrapper = mount(QueryView)
    const queryStore = useQueryStore()
    const errorMessage = 'Syntax error'

    queryStore.errorMessage = errorMessage
    await wrapper.vm.$nextTick()

    const errorPanel = wrapper.find('.error-panel')
    expect(errorPanel.exists()).toBe(true)
    expect(errorPanel.text()).toContain(errorMessage)
  })

  it('calls disconnect on the connection store', async () => {
    const wrapper = mount(QueryView)
    const connectionStore = useConnectionStore()
    const disconnectSpy = vi.spyOn(connectionStore, 'disconnect')

    await wrapper.find('header button').trigger('click')

    expect(disconnectSpy).toHaveBeenCalledTimes(1)
  })
})
