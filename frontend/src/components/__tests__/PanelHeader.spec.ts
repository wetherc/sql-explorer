import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import PanelHeader from '../PanelHeader.vue'
import { mountWithPlugins } from './mount'

describe('PanelHeader', () => {
  it('leaves the filter out when the panel does not filter its content', () => {
    const wrapper = mountWithPlugins(PanelHeader)

    expect(wrapper.find('.panel-filter').exists()).toBe(false)
  })

  it('shows the filter with the name and the test mark it is given', () => {
    const wrapper = mountWithPlugins(PanelHeader, {
      props: {
        filter: 'orders',
        filterPlaceholder: 'Filter objects',
        filterLabel: 'Filter the objects',
        filterTestId: 'explorer-filter',
      },
    })

    const field = wrapper.find('[data-test="explorer-filter"] input')
    expect(field.exists()).toBe(true)
    expect((field.element as HTMLInputElement).value).toBe('orders')
    expect(field.attributes('aria-label')).toBe('Filter the objects')
    expect(field.attributes('placeholder')).toBe('Filter objects')
  })

  it('reports the text the user types', async () => {
    const wrapper = mountWithPlugins(PanelHeader, {
      props: { filter: '', filterTestId: 'panel-filter' },
    })

    await wrapper.find('[data-test="panel-filter"] input').setValue('sales')

    expect(wrapper.emitted('update:filter')).toEqual([['sales']])
  })

  it('reports an empty text when the field is cleared', () => {
    const wrapper = mountWithPlugins(PanelHeader, { props: { filter: 'sales' } })

    wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', null)

    expect(wrapper.emitted('update:filter')).toEqual([['']])
  })

  it('draws the lead and the actions it is given', () => {
    const wrapper = mountWithPlugins(PanelHeader, {
      slots: {
        lead: () => h('button', { 'data-test': 'lead' }, 'New'),
        actions: () => h('button', { 'data-test': 'action' }, 'Refresh'),
      },
    })

    expect(wrapper.find('[data-test="lead"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="action"]').exists()).toBe(true)
  })

  it('holds no switch row unless one is given', () => {
    const bare = mountWithPlugins(PanelHeader)
    expect(bare.findAll('.panel-header-row')).toHaveLength(1)

    const withSwitch = mountWithPlugins(PanelHeader, {
      slots: { switch: () => h('button', { 'data-test': 'switch' }, 'History') },
    })
    expect(withSwitch.findAll('.panel-header-row')).toHaveLength(2)
    expect(withSwitch.find('[data-test="switch"]').exists()).toBe(true)
  })
})
