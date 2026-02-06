import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ConnectionView from '../ConnectionView.vue'
import { useConnectionStore } from '@/stores/connection'

// Mock the tauri invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('ConnectionView.vue', () => {
  beforeEach(() => {
    // Creates a fresh pinia and make it active so it's automatically picked
    // up by any useStore() call without having to pass it to them
    setActivePinia(createPinia())
  })

  it('renders the form correctly', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('h2').text()).toBe('Connect to Database')
    expect(wrapper.find('input').exists()).toBe(true)
    expect(wrapper.find('button').exists()).toBe(true)
  })

  it('calls the connect action on form submission', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')

    const testString = 'server=localhost;user=test'
    await wrapper.find('input').setValue(testString)
    await wrapper.find('form').trigger('submit.prevent')

    expect(connectSpy).toHaveBeenCalledTimes(1)
    expect(connectSpy).toHaveBeenCalledWith(testString)
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

  it('disables the button and shows "Connecting..." while connecting', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()

    connectionStore.isConnecting = true
    await wrapper.vm.$nextTick()

    const button = wrapper.find('button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Connecting...')

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeDefined()
  })
})
