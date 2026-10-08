import { describe, expect, it, vi } from 'vitest'
import { nextTick, reactive } from 'vue'
import type { Message } from '@/types/api'

const QueryMessages = (await import('@/components/QueryMessages.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useQueryStore } = await import('@/stores/query')

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
      props: { tabId: 't1', messages: messages(0), dropped: 0, hasError: false },
    })
    expect(wrapper.find('[data-test="no-messages"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="messages-hidden"]').exists()).toBe(false)
  })

  it('shows no empty note beside a failure', () => {
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { tabId: 't1', messages: messages(0), dropped: 0, hasError: true },
    })
    expect(wrapper.find('[data-test="no-messages"]').exists()).toBe(false)
  })

  it('shows each message that arrives, without a new list', async () => {
    const list = messages(1)
    const wrapper = mountWithPlugins(QueryMessages, {
      props: { tabId: 't1', messages: list, dropped: 0, hasError: false },
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
      props: { tabId: 't1', messages: list, dropped: 0, hasError: false },
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
      props: { tabId: 't1', messages: list, dropped: 1000, hasError: false },
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
      props: { tabId: 't1', messages: messages(3), dropped: 1, hasError: false },
    })
    expect(lines(wrapper)).toHaveLength(3)
    expect(wrapper.find('[data-test="messages-hidden"]').text()).toBe('1 earlier message not shown')
  })

  describe('the save menu', () => {
    /** Mounts the list of tab t1 and opens its save menu. */
    async function openMenu(list: Message[]) {
      const wrapper = mountWithPlugins(QueryMessages, {
        props: { tabId: 't1', messages: list, dropped: 0, hasError: false },
      })
      const queries = useQueryStore()
      const actions = {
        shown: vi.spyOn(queries, 'saveShownMessages').mockResolvedValue(),
        all: vi.spyOn(queries, 'saveAllMessages').mockResolvedValue(),
        stop: vi.spyOn(queries, 'stopSavingMessages').mockReturnValue(),
      }
      await wrapper.find('[data-test="messages-save"]').trigger('click')
      await settle()
      return { wrapper, queries, actions }
    }

    function item(test: string): HTMLElement | null {
      return document.querySelector(`[data-test="${test}"]`)
    }

    it('saves the shown messages or every message of the tab', async () => {
      const { wrapper, actions } = await openMenu(messages(2))
      expect(wrapper.find('[data-test="messages-saving"]').exists()).toBe(false)
      expect(item('messages-stop')).toBeNull()
      expect(item('messages-save-all')!.textContent).toContain('Save all messages to file…')

      item('messages-save-shown')!.click()
      expect(actions.shown).toHaveBeenCalledWith('t1')
      await wrapper.find('[data-test="messages-save"]').trigger('click')
      await settle()
      item('messages-save-all')!.click()
      expect(actions.all).toHaveBeenCalledWith('t1')
    })

    it('offers no save of the shown messages when the list is empty', async () => {
      await openMenu(messages(0))
      expect(item('messages-save-shown')!.className).toContain('v-list-item--disabled')
    })

    it('names the file that gets the messages and lets the user stop', async () => {
      const { wrapper, queries, actions } = await openMenu(messages(1))
      queries.stateFor('t1').messagesFile = { id: 'f1', path: '/logs/run.txt' }
      await nextTick()
      expect(wrapper.find('[data-test="messages-saving"]').text()).toBe(
        'Saving all messages to /logs/run.txt',
      )
      expect(item('messages-save-all')!.textContent).toContain('another file')
      item('messages-stop')!.click()
      expect(actions.stop).toHaveBeenCalledWith('t1')
    })
  })
})
