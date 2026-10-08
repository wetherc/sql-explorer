import { describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const PausedBanner = (await import('@/components/PausedBanner.vue')).default
const { mountWithPlugins, settle } = await import('./mount')

function wait(waitingSession: number) {
  return { waitingSession, blockingSession: 52 }
}

function banner(pause?: Record<string, unknown>) {
  return mountWithPlugins(PausedBanner, {
    props: {
      rows: 100,
      pausedUntil: Date.now() + 60_000,
      exporting: false,
      pause: pause && { connectionId: 'c1', blocking: [], atMost: false, ...pause },
    },
  })
}

describe('PausedBanner', () => {
  it('counts each blocked session once and lists them in the dialog', async () => {
    apiStub.blockingSessions.mockResolvedValue({ sessions: [], openTransactions: [], notes: [] })
    const one = banner({ serverSession: 52, blocking: [wait(57), wait(57)] })
    expect(one.find('[data-test="grid-paused-blocking"]').text()).toBe('Blocking 1 other session')
    const two = banner({ serverSession: 52, blocking: [wait(57), wait(58)] })
    const button = two.find('[data-test="grid-paused-blocking"]')
    expect(button.text()).toBe('Blocking 2 other sessions')
    await button.trigger('click')
    await settle()
    expect(apiStub.blockingSessions).toHaveBeenCalledWith('c1', 52)
    expect(document.body.textContent).toContain('Locks of session 52')
    const close = document.querySelector<HTMLElement>('[data-test="blocking-close"]')!
    close.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(two.findComponent({ name: 'BlockingSessionsDialog' }).exists()).toBe(false)
  })

  it('shows no blocking button and no new tab button without a pause state', () => {
    const plain = banner()
    expect(plain.find('[data-test="grid-paused-blocking"]').exists()).toBe(false)
    expect(plain.find('[data-test="grid-paused-new-tab"]').exists()).toBe(false)
    expect(plain.find('[data-test="grid-paused-server-limit"]').exists()).toBe(false)
  })

  it('emits an extension and a new tab, and stops the extension at the most time', async () => {
    const open = banner({})
    await open.find('[data-test="grid-paused-extend"]').trigger('click')
    await open.find('[data-test="grid-paused-new-tab"]').trigger('click')
    expect(open.emitted('extend')).toHaveLength(1)
    expect(open.emitted('open-tab')).toEqual([['c1']])

    const capped = banner({ atMost: true })
    const extend = capped.find('[data-test="grid-paused-extend"]')
    expect(extend.attributes('disabled')).toBeDefined()
    expect(extend.element.parentElement?.getAttribute('title')).toBe(
      'A query can stay paused for 60 minutes at most.',
    )
  })

  it('names the server limit on idle transactions when it shortens the pause', () => {
    const limit = (serverIdleSecs: number) =>
      banner({ serverIdleSecs }).find('[data-test="grid-paused-server-limit"]')
    expect(limit(300).text()).toBe(
      'The server ends idle transactions after 5 minutes, so the query is released before then.',
    )
    expect(limit(60).text()).toContain('after 1 minute,')
    expect(limit(20).text()).toContain('after 20 seconds,')
    expect(limit(1).text()).toContain('after 1 second,')
    // A limit that leaves the full hour gives no note.
    expect(limit(4000).exists()).toBe(false)
  })
})
