import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ConnectionView from '../ConnectionView.vue'
import { useConnectionStore } from '@/stores/connection'

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('ConnectionView.vue', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders the form with separate fields correctly', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('h2').text()).toBe('Connect to Database')
    expect(wrapper.find('input#server').exists()).toBe(true)
    expect(wrapper.find('input#database').exists()).toBe(true)
    expect(wrapper.find('input#username').exists()).toBe(true)
    expect(wrapper.find('input#password').exists()).toBe(true)
    expect(wrapper.find('button[type="submit"]').exists()).toBe(true)
  })

  it('calls the connect action with a constructed connection string', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')

    // Set values
    await wrapper.find('input#server').setValue('test-server')
    await wrapper.find('input#database').setValue('test-db')
    await wrapper.find('input#username').setValue('test-user')
    await wrapper.find('input#password').setValue('test-pass')

    // Submit form
    await wrapper.find('form').trigger('submit.prevent')

    const expectedConnectionString =
      'server=test-server;database=test-db;user=test-user;password=test-pass;TrustServerCertificate=true'

    expect(connectSpy).toHaveBeenCalledTimes(1)
    expect(connectSpy).toHaveBeenCalledWith(expectedConnectionString)
  })

  it('shows an error message when the store has an error', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()
    const errorMessage = 'Connection failed'

    connectionStore.errorMessage = errorMessage
    await wrapper.vm.$nextTick()

    const errorP = wrapper.find('p.error-message')
    expect(errorP.exists()).toBe(true)
    expect(errorP.text()).toBe(errorMessage)
  })

  it('disables fields and shows "Connecting..." while connecting', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()

    connectionStore.isConnecting = true
    await wrapper.vm.$nextTick()

    const button = wrapper.find('button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Connecting...')

    const inputs = wrapper.findAll('input')
    inputs.forEach((input) => {
      expect(input.attributes('disabled')).toBeDefined()
    })
  })
})
