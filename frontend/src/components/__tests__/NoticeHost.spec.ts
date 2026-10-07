import { describe, expect, it, vi } from 'vitest'
import NoticeHost from '@/components/NoticeHost.vue'
import { mountWithPlugins, settle } from './mount'
import { useUiStore } from '@/stores/ui'
import { ErrorCategory } from '@/types/api'

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
      category: ErrorCategory.Database,
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
    vi.useFakeTimers()
    try {
      ui.success('gone soon')
      ui.reportError(new Error('stays'))
      await wrapper.vm.$nextTick()
      vi.advanceTimersByTime(3000)
      expect(ui.notices.map((notice) => notice.message)).toEqual(['stays'])
    } finally {
      vi.useRealTimers()
    }
    wrapper.unmount()
  })

  it('stops the timer of a notice that the user took away', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    vi.useFakeTimers()
    try {
      const first = ui.info('first')
      await wrapper.vm.$nextTick()
      ui.dismiss(first.id)
      await wrapper.vm.$nextTick()
      const dismiss = vi.spyOn(ui, 'dismiss')
      vi.advanceTimersByTime(5000)
      expect(dismiss).not.toHaveBeenCalled()
      ui.info('second')
      await wrapper.vm.$nextTick()
      wrapper.unmount()
      vi.advanceTimersByTime(5000)
      expect(dismiss).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })

  it('stacks the notices in one column with the newest at the top', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.warn('older')
    ui.warn('newer')
    await settle()
    const texts = wrapper.findAll('[data-test="notice"]').map((notice) => notice.text())
    expect(texts[0]).toContain('newer')
    expect(texts[1]).toContain('older')
    expect(wrapper.find('[data-test="notice"] .notice-text').attributes('title')).toBe('newer')
    wrapper.unmount()
  })
})

describe('NoticeHost details', () => {
  it('offers details for an error that has no detail of its own', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Database, message: 'plain failure', detail: null })
    await settle()
    expect(ui.notices[0]!.detail).toBeNull()
    await wrapper.find('[data-test="notice-details"]').trigger('click')
    await settle()
    expect(document.querySelector('[data-test="notice-detail-body"]')).toBeNull()
    expect(document.body.textContent).toContain('plain failure')
    wrapper.unmount()
  })

  it('copies the message and the detail', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Database, message: 'no', detail: 'why' })
    ui.openNotice(ui.notices[0]!)
    await settle()
    ;(document.querySelector('[data-test="notice-copy"]') as HTMLElement).click()
    await settle()
    expect(writeText).toHaveBeenCalledWith(expect.stringMatching(/^no\n\n.*why/s))
    expect(ui.notices.some((notice) => notice.level === 'success')).toBe(true)
    wrapper.unmount()
  })

  it('reports a copy that the clipboard refused', async () => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
    })
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.openNotice(ui.warn('a note', 'more'))
    await settle()
    ;(document.querySelector('[data-test="notice-copy"]') as HTMLElement).click()
    await settle()
    expect(ui.notices.some((notice) => notice.message === 'denied')).toBe(true)
    wrapper.unmount()
  })

  it('warns when there is no clipboard', async () => {
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: undefined,
    })
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.openNotice(ui.warn('a note'))
    await settle()
    ;(document.querySelector('[data-test="notice-copy"]') as HTMLElement).click()
    await settle()
    expect(ui.notices.some((notice) => notice.message.includes("wasn't copied"))).toBe(true)
    wrapper.unmount()
  })
})

describe('NoticeHost dialog', () => {
  it('closes the dialog from its own button', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Database, message: 'no', detail: 'why' })
    await settle()

    const details = document.querySelector('[data-test="notice-details"]') as HTMLElement
    details.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    const close = [...document.querySelectorAll('.v-card-actions .v-btn')].find((button) =>
      button.textContent?.includes('Close'),
    )
    close?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    await settle()
    expect(ui.openedNotice).toBeNull()
    wrapper.unmount()
  })

  it('closes the dialog when the overlay reports it', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Database, message: 'no', detail: 'why' })
    await settle()
    ui.openNotice(ui.notices[0]!)
    await settle()

    const dialog = wrapper.findComponent({ name: 'VDialog' })
    await dialog.vm.$emit('update:modelValue', false)
    // The dialog moves the focus back on the next tick, so it must still
    // be mounted when that runs.
    await settle()
    expect(ui.openedNotice).toBeNull()
    wrapper.unmount()
  })
})

describe('NoticeHost as a part a reader can follow', () => {
  it('puts no live region inside another one', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    useUiStore().reportError(new Error('It failed'))
    await settle()
    const alert = document.querySelector('[role="alert"]')!
    expect(alert.parentElement?.closest('[aria-live], [role="status"], [role="alert"]')).toBeNull()
    wrapper.unmount()
  })

  it('breaks in for an error and waits its turn for anything else', async () => {
    const wrapper = mountWithPlugins(NoticeHost)
    const ui = useUiStore()

    ui.success('Saved')
    ui.reportError(new Error('It failed'))
    await settle()

    const parts = [...document.querySelectorAll('[role="status"], [role="alert"]')]
    const roles = parts.map((part) => part.getAttribute('role'))
    expect(roles).toContain('status')
    expect(roles).toContain('alert')
    const alert = parts.find((part) => part.getAttribute('role') === 'alert')
    expect(alert?.getAttribute('aria-live')).toBe('assertive')
    wrapper.unmount()
  })
})

describe('NoticeHost taking many notices away at once', () => {
  it('offers no button while one notice stands alone', async () => {
    mountWithPlugins(NoticeHost)
    useUiStore().info('One')
    await settle()

    expect(document.querySelector('[data-test="notice-clear"]')).toBeNull()
  })

  it('offers one button once a second notice arrives', async () => {
    mountWithPlugins(NoticeHost)
    const ui = useUiStore()
    ui.info('One')
    ui.info('Two')
    await settle()

    expect(document.body.textContent).toContain('2 notices')
    const clear = document.querySelector('[data-test="notice-clear"]') as HTMLElement
    clear.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(ui.notices).toHaveLength(0)
  })
})
