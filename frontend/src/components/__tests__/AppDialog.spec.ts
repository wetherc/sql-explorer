import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import AppDialog from '../AppDialog.vue'
import { mountWithPlugins, settle } from './mount'
import { useUiStore } from '@/stores/ui'

describe('AppDialog', () => {
  it('counts itself while it stands open', async () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: false } })
    const ui = useUiStore()
    expect(ui.dialogOpen).toBe(false)

    await wrapper.setProps({ modelValue: true })
    expect(ui.openDialogs).toBe(1)
    expect(ui.dialogOpen).toBe(true)

    await wrapper.setProps({ modelValue: false })
    expect(ui.openDialogs).toBe(0)
    expect(ui.dialogOpen).toBe(false)
  })

  it('counts itself once, however often the same state arrives', async () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: true } })
    const ui = useUiStore()

    await wrapper.setProps({ modelValue: true })

    expect(ui.openDialogs).toBe(1)
  })

  it('takes itself off the count when it goes away while open', () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: true } })
    const ui = useUiStore()
    expect(ui.openDialogs).toBe(1)

    wrapper.unmount()

    expect(ui.openDialogs).toBe(0)
  })

  it('reports the state the dialog below it gives back', async () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: true } })

    await wrapper.findComponent({ name: 'VDialog' }).vm.$emit('update:modelValue', false)

    expect(wrapper.emitted('update:modelValue')).toEqual([[false]])
  })

  it('draws what it is given, in the dialog below it', async () => {
    mountWithPlugins(AppDialog, {
      props: { modelValue: true },
      slots: { default: () => h('p', { 'data-test': 'body' }, 'Inside') },
    })
    await settle()

    expect(document.querySelector('[data-test="body"]')?.textContent).toBe('Inside')
  })
})
