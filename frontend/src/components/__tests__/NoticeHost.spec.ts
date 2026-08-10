import { describe, expect, it } from 'vitest'
import NoticeHost from '@/components/NoticeHost.vue'
import { mountWithPlugins, settle } from './mount'
import { useUiStore } from '@/stores/ui'
import { ErrorKind } from '@/types/api'

describe('NoticeHost', () => {
  it('draws nothing when there is no notice', () => {
    const wrapper = mountWithPlugins(NoticeHost)
    expect(wrapper.findAll('[data-test="notice"]')).toHaveLength(0)
  })

  it('shows a notice and removes it on request', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.success('The connection is saved.')
    await settle()

    expect(document.body.textContent).toContain('The connection is saved.')

    const close = document.querySelector('[data-test="notice-close"]') as HTMLElement
    close.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(ui.notices).toHaveLength(0)
    wrapper.unmount()
  })

  it('opens the whole reason of a failure', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.reportError({
      kind: ErrorKind.Database,
      message: 'no such column',
      detail: 'line 1, column 8',
    })
    await settle()

    const details = document.querySelector('[data-test="notice-details"]') as HTMLElement
    details.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(document.body.textContent).toContain('line 1, column 8')
    ui.closeNotice()
    await settle()
    expect(ui.openedNotice).toBeNull()
    wrapper.unmount()
  })

  it('offers no details for a notice that carries none', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    useUiStore().info('a note')
    await settle()
    expect(document.querySelector('[data-test="notice-details"]')).toBeNull()
    wrapper.unmount()
  })

  it('removes a notice when it runs out of time', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.success('gone soon')
    await settle()
    await wrapper.findComponent({ name: 'VSnackbar' }).vm.$emit('update:modelValue', false)
    expect(ui.notices).toHaveLength(0)
    wrapper.unmount()
  })
})
