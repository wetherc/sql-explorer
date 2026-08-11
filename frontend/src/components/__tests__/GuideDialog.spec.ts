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
    expect(body).toContain(last.body.slice(0, 20))
  })

  it('reports that the reader closed it', async () => {
    const wrapper = await mountGuide()

    const close = document.querySelector('[data-test="guide-close"]') as HTMLElement
    close.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })
})
