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

  it('renders the auth type selector', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('select#auth-type').exists()).toBe(true)
  })

  it('shows username/password fields for SQL Server Auth by default', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('input#username').exists()).toBe(true)
    expect(wrapper.find('input#password').exists()).toBe(true)
  })

  it('hides username/password fields for Integrated Auth', async () => {
    const wrapper = mount(ConnectionView)
    const selector = wrapper.find('select#auth-type')
    await selector.setValue('integrated')

    expect(wrapper.find('input#username').exists()).toBe(false)
    expect(wrapper.find('input#password').exists()).toBe(false)
  })

  it('builds a SQL Server Auth connection string', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')

    await wrapper.find('input#server').setValue('test-server')
    await wrapper.find('input#database').setValue('test-db')
    await wrapper.find('select#auth-type').setValue('sql')
    await wrapper.find('input#username').setValue('test-user')
    await wrapper.find('input#password').setValue('test-pass')

    await wrapper.find('form').trigger('submit.prevent')

    const expected =
      'server=test-server;database=test-db;TrustServerCertificate=true;user=test-user;password=test-pass'
    expect(connectSpy).toHaveBeenCalledWith(expected)
  })

  it('builds an Integrated Auth connection string', async () => {
    const wrapper = mount(ConnectionView)
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')

    await wrapper.find('input#server').setValue('test-server')
    await wrapper.find('input#database').setValue('test-db')
    await wrapper.find('select#auth-type').setValue('integrated')

    await wrapper.find('form').trigger('submit.prevent')

    const expected =
      'server=test-server;database=test-db;TrustServerCertificate=true;Authentication=ActiveDirectoryIntegrated'
    expect(connectSpy).toHaveBeenCalledWith(expected)
  })
})
