import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ConnectionView from '../ConnectionView.vue'

describe('ConnectionView.vue', () => {
  it('renders the form correctly', () => {
    const wrapper = mount(ConnectionView)
    expect(wrapper.find('h2').text()).toBe('Connect to Database')
    expect(wrapper.find('label[for="connection-string"]').exists()).toBe(true)
    expect(wrapper.find('input#connection-string').exists()).toBe(true)
    expect(wrapper.find('button[type="submit"]').exists()).toBe(true)
  })

  it('updates the connection string ref on input', async () => {
    const wrapper = mount(ConnectionView)
    const input = wrapper.find('input#connection-string')
    const testString = 'server=localhost'
    await input.setValue(testString)
    expect(wrapper.vm.connectionString).toBe(testString)
  })

  it('does not emit a connect event if the connection string is empty', async () => {
    const wrapper = mount(ConnectionView)
    await wrapper.find('form').trigger('submit.prevent')
    expect(wrapper.emitted('connect')).toBeUndefined()
  })

  it('emits a connect event with the connection string on form submission', async () => {
    const wrapper = mount(ConnectionView)
    const input = wrapper.find('input#connection-string')
    const testString = 'server=localhost;user=test'
    await input.setValue(testString)
    await wrapper.find('form').trigger('submit.prevent')

    expect(wrapper.emitted()).toHaveProperty('connect')
    const connectEvent = wrapper.emitted('connect')
    expect(connectEvent).toHaveLength(1)
    expect(connectEvent[0]).toEqual([testString])
  })
})
