import { describe, expect, it } from 'vitest'
import { nextTick, reactive } from 'vue'
import type { Message } from '@/types/api'

const QueryMessages = (await import('@/components/QueryMessages.vue')).default
const { mountWithPlugins } = await import('./mount')

/** A list of messages as the store keeps it, with numbered texts. */
function messages(count: number): Message[] {
  return reactive(
    Array.from({ length: count }, (_, index) => ({
      level: 'info' as const,
      text: `line ${index + 1}`,
      detail: null,
    })),
  )
}

function lines(wrapper: ReturnType<typeof mountWithPlugins>): string[] {
  return wrapper.findAll('[data-test="query-message"]').map((line) => line.text())
}

describe('QueryMessages', () => {
  it('says so when there is no message and no failure', () => {
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: messages(0), dropped: 0, hasError: false },
    })
    expect(wrapper.find('[data-test="no-messages"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="messages-hidden"]').exists()).toBe(false)
  })

  it('shows no empty note beside a failure', () => {
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: messages(0), dropped: 0, hasError: true },
    })
    expect(wrapper.find('[data-test="no-messages"]').exists()).toBe(false)
  })

  it('shows each message that arrives, without a new list', async () => {
    const list = messages(1)
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: list, dropped: 0, hasError: false },
    })
    expect(lines(wrapper)).toEqual(['line 1'])

    list.push({ level: 'warning', text: 'careful', detail: 'more' })
    list.push({ level: 'error', text: 'failed', detail: null })
    await nextTick()
    expect(lines(wrapper)).toEqual([
      'line 1',
      expect.stringContaining('careful'),
      expect.stringContaining('failed'),
    ])
    expect(wrapper.find('.message-detail').text()).toBe('more')
    expect(wrapper.find('.message-error').html()).toContain('mdi-alert-circle')
  })

  it('draws the last messages alone and counts the others', async () => {
    const list = messages(501)
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: list, dropped: 0, hasError: false },
    })
    expect(lines(wrapper)).toHaveLength(500)
    expect(lines(wrapper)[0]).toBe('line 2')
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe('1 earlier message not shown')

    list.push({ level: 'info', text: 'line 502', detail: null })
    await nextTick()
    expect(lines(wrapper)).toHaveLength(500)
    expect(lines(wrapper)[0]).toBe('line 3')
    expect(lines(wrapper)[499]).toBe('line 502')
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe(
      '2 earlier messages not shown',
    )
  })

  it('counts the messages that the store dropped with the hidden ones', async () => {
    const list = messages(600)
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: list, dropped: 1000, hasError: false },
    })
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe(
      '1,100 earlier messages not shown',
    )
    const first = wrapper.find('[data-test="query-message"]').element

    // The store drops the first 100 messages of its list. The lines on
    // screen are the same, so each line keeps its element.
    await wrapper.setProps({ messages: reactive(list.slice(100)), dropped: 1100 })
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe(
      '1,100 earlier messages not shown',
    )
    expect(lines(wrapper)[0]).toBe('line 101')
    expect(wrapper.find('[data-test="query-message"]').element).toBe(first)
  })

  it('counts dropped messages when the list is short', () => {
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { messages: messages(3), dropped: 1, hasError: false },
    })
    expect(lines(wrapper)).toHaveLength(3)
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe('1 earlier message not shown')
  })
})
