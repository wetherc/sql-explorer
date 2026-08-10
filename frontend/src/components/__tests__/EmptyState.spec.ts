import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import EmptyState from '../EmptyState.vue'
import { mountWithPlugins } from './mount'

describe('EmptyState', () => {
  it('shows the icon and the title of a panel state', () => {
    const wrapper = mountWithPlugins(EmptyState, {
      props: { icon: 'mdi-database-off-outline', title: 'No open connection' },
    })

    expect(wrapper.text()).toContain('No open connection')
    expect(wrapper.find('.mdi-database-off-outline').exists()).toBe(true)
    expect(wrapper.find('.empty-state--panel').exists()).toBe(true)
    expect(wrapper.find('.text-body-2.empty-state-title').exists()).toBe(true)
  })

  it('leaves the hint out when none is given', () => {
    const wrapper = mountWithPlugins(EmptyState, {
      props: { icon: 'mdi-history', title: 'Nothing yet' },
    })

    expect(wrapper.find('.empty-state-hint').exists()).toBe(false)
  })

  it('shows the hint when one is given', () => {
    const wrapper = mountWithPlugins(EmptyState, {
      props: { icon: 'mdi-history', title: 'Nothing yet', hint: 'Run a statement.' },
    })

    expect(wrapper.find('.empty-state-hint').text()).toBe('Run a statement.')
  })

  it('makes a page state larger than a panel state', () => {
    const wrapper = mountWithPlugins(EmptyState, {
      props: { icon: 'mdi-database-search-outline', title: 'No open tabs', size: 'page' },
    })

    expect(wrapper.find('.empty-state--page').exists()).toBe(true)
    expect(wrapper.find('.text-h6.empty-state-title').exists()).toBe(true)
  })

  it('draws the action it is given', () => {
    const wrapper = mountWithPlugins(EmptyState, {
      props: { icon: 'mdi-plus', title: 'Nothing yet' },
      slots: { default: () => h('button', { 'data-test': 'action' }, 'Add') },
    })

    expect(wrapper.find('[data-test="action"]').exists()).toBe(true)
  })
})
