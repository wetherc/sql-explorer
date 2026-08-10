import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import ConfirmDialog from '../ConfirmDialog.vue'
import { mountWithPlugins, settle } from './mount'

async function mountConfirm(props: Record<string, unknown> = {}, slots = {}) {
  const wrapper = mountWithPlugins(ConfirmDialog, {
    props: { open: true, title: 'Delete this?', ...props },
    slots,
  })
  await settle()
  return wrapper
}

describe('ConfirmDialog', () => {
  it('shows the question and the message it is given', async () => {
    await mountConfirm({ message: 'The record goes away.' })

    expect(document.body.textContent).toContain('Delete this?')
    expect(document.body.textContent).toContain('The record goes away.')
  })

  it('lets a caller give the body of the question in place of a message', async () => {
    await mountConfirm({ message: 'Plain words' }, { default: () => h('p', 'Rich words') })

    expect(document.body.textContent).toContain('Rich words')
    expect(document.body.textContent).not.toContain('Plain words')
  })

  it('names its two buttons, and takes other names when it is given them', async () => {
    await mountConfirm()
    expect(document.querySelector('[data-test="confirm-accept"]')?.textContent).toContain('Yes')
    expect(document.querySelector('[data-test="confirm-cancel"]')?.textContent).toContain('Cancel')

    await mountConfirm({ confirmText: 'Delete', cancelText: 'Keep it' })
    expect(document.querySelector('[data-test="confirm-accept"]')?.textContent).toContain('Delete')
    expect(document.querySelector('[data-test="confirm-cancel"]')?.textContent).toContain('Keep it')
  })

  it('gives the button that acts the colour of what it does', async () => {
    await mountConfirm()
    expect(document.querySelector('[data-test="confirm-accept"]')?.className).toContain(
      'bg-primary',
    )

    await mountConfirm({ danger: true })
    expect(document.querySelector('[data-test="confirm-accept"]')?.className).toContain('bg-error')
  })

  it('reports the answer of the user', async () => {
    const wrapper = await mountConfirm()

    const accept = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    accept.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(wrapper.emitted('confirm')).toHaveLength(1)

    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('reports a refusal when the overlay closes itself', async () => {
    const wrapper = await mountConfirm()

    await wrapper.findComponent({ name: 'AppDialog' }).vm.$emit('update:modelValue', false)

    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('draws nothing while it is shut', async () => {
    await mountConfirm({ open: false })

    expect(document.querySelector('[data-test="confirm-accept"]')).toBeNull()
  })
})
