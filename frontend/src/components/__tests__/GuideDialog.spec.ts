import { describe, expect, it } from 'vitest'
import GuideDialog from '@/components/GuideDialog.vue'
import { GUIDE_TOPICS } from '@/lib/guide'
import { mountWithPlugins, settle } from './mount'

async function mountGuide(open = true) {
  const wrapper = mountWithPlugins(GuideDialog, { props: { open } })
  await settle()
  return wrapper
}

describe('GuideDialog', () => {
  it('shows nothing while it is closed', async () => {
    await mountGuide(false)
    expect(document.querySelector('[data-test="guide-content"]')).toBeNull()
  })

  it('opens on the first topic and lists every topic', async () => {
    await mountGuide()

    const first = GUIDE_TOPICS[0]!
    expect(document.querySelector('[data-test="guide-content"]')?.textContent).toContain(
      first.title,
    )
    for (const topic of GUIDE_TOPICS) {
      expect(document.querySelector(`[data-test="guide-topic-${topic.id}"]`)).not.toBeNull()
    }
  })

  it('shows the topic that the reader chose', async () => {
    await mountGuide()
    const last = GUIDE_TOPICS[GUIDE_TOPICS.length - 1]!

    const item = document.querySelector(`[data-test="guide-topic-${last.id}"]`) as HTMLElement
    item.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    const body = document.querySelector('[data-test="guide-content"]')?.textContent
    expect(body).toContain(last.title)
  })

  it('draws the text of a topic as HTML and not as Markdown', async () => {
    await mountGuide()

    const text = document.querySelector('.topic-text')
    expect(text?.innerHTML).toContain('<p>')
    // The marks of Markdown are gone from the text that the reader sees.
    expect(text?.textContent).not.toContain('**')
  })

  it('holds its height on the dialog, so a topic cannot change it', async () => {
    const wrapper = await mountGuide()

    // A scrollable dialog makes the card a flex item with a basis of the
    // whole height, and that basis wins over a height of the card. The
    // height therefore belongs to the dialog.
    expect(wrapper.findComponent({ name: 'VDialog' }).props('height')).toBe('70vh')

    const content = document.querySelector('.v-overlay__content') as HTMLElement
    expect(content.style.height).toBe('70vh')
  })

  it('reports that the reader closed it', async () => {
    const wrapper = await mountGuide()

    const close = document.querySelector('[data-test="guide-close"]') as HTMLElement
    close.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })
})
