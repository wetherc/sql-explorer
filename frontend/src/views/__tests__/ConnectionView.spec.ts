import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ConnectionView from '../ConnectionView.vue'
import { useConnectionStore } from '@/stores/connection'
import * as connBuilder from '@/utils/connectionStringBuilder'

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('ConnectionView.vue', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders all form fields correctly', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('input#server').exists()).toBe(true)
    expect(wrapper.find('input#database').exists()).toBe(true)
    expect(wrapper.find('select#auth-type').exists()).toBe(true)
    expect(wrapper.find('input#username').exists()).toBe(true)
    expect(wrapper.find('input#password').exists()).toBe(true)
  })

  it('calls the connection string builder and store action on submit', async () => {
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    const builderSpy = vi
      .spyOn(connBuilder, 'buildConnectionString')
      .mockReturnValue('fake-connection-string')

    const wrapper = mount(ConnectionView)

    // Set form values
    await wrapper.find('input#server').setValue('test-server')
    await wrapper.find('input#database').setValue('test-db')
    await wrapper.find('select#auth-type').setValue('sql')
    await wrapper.find('input#username').setValue('test-user')
    await wrapper.find('input#password').setValue('test-pass')

    // Submit
    await wrapper.find('form').trigger('submit.prevent')

    // Check that the builder was called with the correct form data
    expect(builderSpy).toHaveBeenCalledTimes(1)
    expect(builderSpy).toHaveBeenCalledWith({
      server: 'test-server',
      database: 'test-db',
      authType: 'sql',
      username: 'test-user',
      password: 'test-pass',
    })

    // Check that the store action was called with the result from the builder
    expect(connectSpy).toHaveBeenCalledTimes(1)
    expect(connectSpy).toHaveBeenCalledWith('fake-connection-string')
  })

  it('displays an error if the builder throws an error', async () => {
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    vi.spyOn(connBuilder, 'buildConnectionString').mockImplementation(() => {
      throw new Error('Invalid server name')
    })

    const wrapper = mount(ConnectionView)
    await wrapper.find('form').trigger('submit.prevent')

    // Ensure the connect action was NOT called
    expect(connectSpy).not.toHaveBeenCalled()

    // Ensure the error message is displayed
    expect(wrapper.find('.error-message').exists()).toBe(true)
    expect(wrapper.find('.error-message').text()).toBe('Invalid server name')
    expect(connectionStore.errorMessage).toBe('Invalid server name')
  })
})
