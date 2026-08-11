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

describe('AppDialog and the focus of the user', () => {
  /** Puts a button in the document and gives it the focus. */
  function opener(): HTMLButtonElement {
    const button = document.createElement('button')
    document.body.append(button)
    button.focus()
    return button
  }

  it('gives the focus back to whatever opened it', async () => {
    // The dialog is mounted first, because mounting empties the document.
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: false } })
    const button = opener()

    await wrapper.setProps({ modelValue: true })
    await settle()
    ;(document.activeElement as HTMLElement)?.blur()

    await wrapper.setProps({ modelValue: false })
    await settle()

    expect(document.activeElement).toBe(button)
    button.remove()
  })

  it('gives the focus to nothing when the opener has gone', async () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: false } })
    const button = opener()

    await wrapper.setProps({ modelValue: true })
    await settle()
    button.remove()

    await wrapper.setProps({ modelValue: false })
    await settle()

    expect(document.activeElement).not.toBe(button)
  })

  it('holds no opener when the focus stood on nothing', async () => {
    const wrapper = mountWithPlugins(AppDialog, { props: { modelValue: false } })
    ;(document.activeElement as HTMLElement)?.blur()

    await wrapper.setProps({ modelValue: true })
    await settle()
    await wrapper.setProps({ modelValue: false })
    await settle()

    expect(document.activeElement).toBe(document.body)
  })
})
