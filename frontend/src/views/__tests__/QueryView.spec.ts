import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import QueryView from '../QueryView.vue'
import { useQueryStore } from '@/stores/query'
import { useConnectionStore } from '@/stores/connection'
import MonacoEditor from 'monaco-editor-vue3'
import Message from 'primevue/message'

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('QueryView.vue', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  const defaultProps = {
    query: '',
    response: undefined,
    isLoading: false,
    errorMessage: null,
  }

  it('renders the Monaco Editor component', () => {
    const wrapper = mount(QueryView, { props: { ...defaultProps } })
    expect(wrapper.findComponent(MonacoEditor).exists()).toBe(true)
  })

  it('calls executeQuery action on form submission', async () => {
    const wrapper = mount(QueryView, { props: { ...defaultProps, query: 'SELECT * FROM users' } })
    const queryStore = useQueryStore()
    const executeSpy = vi.spyOn(queryStore, 'executeQuery')
    const testQuery = 'SELECT * FROM users'

    // Directly set the data ref instead of simulating editor input
    await wrapper.findComponent(MonacoEditor).trigger('update:modelValue', testQuery)
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted('execute-query')).toBeTruthy()
  })

  it('displays results in a table', async () => {
    const results = [
      { id: 1, name: 'Alice' },
      { id: 2, name: 'Bob' },
    ]
    const wrapper = mount(QueryView, {
      props: { ...defaultProps, response: { results: [{ rows: results, columns: ['id', 'name'] }], messages: [] } },
    })
    await wrapper.vm.$nextTick()

    const table = wrapper.find('table')
    expect(table.exists()).toBe(true)
    const headers = table.findAll('th')
    expect(headers.length).toBe(2)
    const rows = table.findAll('tbody tr')
    expect(rows.length).toBe(2)
  })

  it('displays an error message', async () => {
    const errorMessage = 'Syntax error'
    const wrapper = mount(QueryView, { props: { ...defaultProps, errorMessage } })
    const queryStore = useQueryStore()

    queryStore.errorMessage = errorMessage
    await wrapper.vm.$nextTick()

    const messageComponent = wrapper.findComponent(Message)
    expect(messageComponent.exists()).toBe(true)
    expect(messageComponent.text()).toContain(errorMessage)
  })

  // it('calls disconnect on the connection store', async () => {
  //   const wrapper = mount(QueryView, { props: { ...defaultProps } })
  //   const connectionStore = useConnectionStore()
  //   const disconnectSpy = vi.spyOn(connectionStore, 'disconnect')

  //   await wrapper.find('header button').trigger('click')

  //   expect(disconnectSpy).toHaveBeenCalledTimes(1)
  // })
})
