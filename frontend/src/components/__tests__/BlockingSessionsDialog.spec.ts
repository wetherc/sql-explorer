import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const BlockingSessionsDialog = (await import('@/components/BlockingSessionsDialog.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useUiStore } = await import('@/stores/ui')

/** The dialog draws into the overlay of the page, so the tests read that. */
function overlayText(): string {
  return document.body.textContent ?? ''
}

function overlayAll(test: string): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>(`[data-test="${test}"]`)]
}

function lastNotice() {
  const notices = useUiStore().notices
  return notices[notices.length - 1]
}

function click(test: string, index = 0): void {
  overlayAll(test)[index]!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
}

const fullWait = {
  waitingSession: 57,
  waitingStatement: 'SELECT * FROM dbo.orders',
  waitMs: 1500,
  blockingSession: 52,
  blockingLogin: 'sa',
  blockingHost: 'build-01',
  blockingProgram: 'sqlcmd',
  blockingStatus: 'sleeping',
  blockingStatement: 'UPDATE dbo.orders SET total = 0',
  object: 'dbo.orders',
  lockMode: 'LCK_M_S',
}

const bareWait = {
  waitingSession: 58,
  waitingStatement: null,
  waitMs: null,
  blockingSession: 52,
  blockingLogin: null,
  blockingHost: null,
  blockingProgram: null,
  blockingStatus: null,
  blockingStatement: null,
  object: null,
  lockMode: null,
}

const report = {
  sessions: [fullWait, bareWait],
  openTransactions: [
    {
      session: 52,
      login: 'sa',
      host: 'build-01',
      program: 'sqlcmd',
      status: 'sleeping',
      statement: 'UPDATE dbo.orders SET total = 0',
      openSecs: 90,
    },
    {
      session: 60,
      login: null,
      host: null,
      program: null,
      status: null,
      statement: null,
      openSecs: null,
    },
  ],
  notes: ['Your login lacks VIEW SERVER STATE, so the report lists your own sessions only.'],
}

function mountDialog(props: Record<string, unknown> = {}) {
  return mountWithPlugins(BlockingSessionsDialog, {
    props: { open: true, connectionId: 'c1', ...props },
  })
}

describe('BlockingSessionsDialog', () => {
  const clipboard = Object.getOwnPropertyDescriptor(globalThis.navigator, 'clipboard')

  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
  })

  afterEach(() => {
    if (clipboard) {
      Object.defineProperty(globalThis.navigator, 'clipboard', clipboard)
    } else {
      Reflect.deleteProperty(globalThis.navigator, 'clipboard')
    }
  })

  it('lists each wait, each open transaction and each note', async () => {
    apiStub.blockingSessions.mockResolvedValue(report)
    mountDialog()
    await settle()

    expect(apiStub.blockingSessions).toHaveBeenCalledWith('c1', null)
    expect(overlayText()).toContain('Blocking sessions')
    expect(overlayAll('blocking-note')[0]?.textContent).toContain('VIEW SERVER STATE')

    const waits = overlayAll('blocking-wait')
    expect(waits).toHaveLength(2)
    expect(waits[0]!.textContent).toContain('Session 57 is waiting for session 52')
    expect(waits[0]!.textContent).toContain('LCK_M_S')
    expect(waits[0]!.textContent).toContain('1.50 s')
    expect(waits[0]!.textContent).toContain('Blocking statement')
    expect(waits[0]!.textContent).toContain('Waiting statement')
    // A wait with no facts shows its line alone.
    expect(waits[1]!.querySelectorAll('dt')).toHaveLength(0)
    expect(waits[1]!.querySelectorAll('pre')).toHaveLength(0)

    const transactions = overlayAll('blocking-transaction')
    expect(transactions).toHaveLength(2)
    expect(transactions[0]!.textContent).toContain('1 min 30 s')
    expect(transactions[0]!.textContent).toContain('Last statement')
    expect(transactions[1]!.querySelectorAll('dt')).toHaveLength(0)
    expect(transactions[1]!.querySelectorAll('pre')).toHaveLength(0)
    expect(overlayAll('no-waits')).toHaveLength(0)
    expect(overlayAll('no-transactions')).toHaveLength(0)
  })

  it('says so when nothing waits and no transaction is open', async () => {
    apiStub.blockingSessions.mockResolvedValue({ sessions: [], openTransactions: [], notes: [] })
    mountDialog({ blockedBy: 52 })
    await settle()

    expect(apiStub.blockingSessions).toHaveBeenCalledWith('c1', 52)
    expect(overlayText()).toContain('Locks of session 52')
    expect(overlayAll('no-waits')).toHaveLength(1)
    expect(overlayAll('no-transactions')).toHaveLength(1)
  })

  it('reads nothing while it stands closed', async () => {
    mountDialog({ open: false })
    await settle()
    expect(apiStub.blockingSessions).not.toHaveBeenCalled()
  })

  it('shows a failure in the dialog and reads again on Refresh', async () => {
    apiStub.blockingSessions.mockRejectedValueOnce({
      category: 'database',
      message: 'Permission denied.',
      detail: 'Error 300',
    })
    const wrapper = mountDialog()
    await settle()
    const alert = overlayAll('blocking-error')[0]
    expect(alert?.textContent).toContain('Permission denied.')
    expect(alert?.textContent).toContain('Error 300')
    expect(useUiStore().notices).toHaveLength(0)

    apiStub.blockingSessions.mockRejectedValueOnce({
      category: 'database',
      message: 'Still denied.',
      detail: null,
    })
    click('blocking-refresh')
    await settle()
    expect(overlayAll('blocking-error')[0]?.querySelector('pre')).toBeNull()

    apiStub.blockingSessions.mockResolvedValue(report)
    click('blocking-refresh')
    await settle()
    expect(overlayAll('blocking-error')).toHaveLength(0)
    expect(overlayAll('blocking-wait')).toHaveLength(2)

    click('blocking-close')
    expect(wrapper.emitted('close')).toHaveLength(1)
    // Escape and a click outside close the dialog too.
    wrapper.findComponent({ name: 'AppDialog' }).vm.$emit('update:modelValue', false)
    expect(wrapper.emitted('close')).toHaveLength(2)
  })

  it('drops a slow answer and a slow failure that a newer read passed', async () => {
    let answerFirst: (value: unknown) => void = () => {}
    let failSecond: (error: unknown) => void = () => {}
    apiStub.blockingSessions
      .mockImplementationOnce(() => new Promise((resolve) => (answerFirst = resolve)))
      .mockImplementationOnce(() => new Promise((_, reject) => (failSecond = reject)))
      .mockResolvedValue({ sessions: [], openTransactions: [], notes: [] })
    const wrapper = mountDialog()
    await settle()
    await wrapper.setProps({ blockedBy: 52 })
    await settle()
    await wrapper.setProps({ blockedBy: 53 })
    await settle()
    expect(overlayAll('no-waits')).toHaveLength(1)

    answerFirst(report)
    failSecond({ category: 'database', message: 'late', detail: null })
    await settle()
    expect(overlayAll('blocking-wait')).toHaveLength(0)
    expect(overlayAll('blocking-error')).toHaveLength(0)
    expect(overlayAll('blocking-loading')).toHaveLength(0)
  })

  it('copies a statement', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    apiStub.blockingSessions.mockResolvedValue(report)
    mountDialog()
    await settle()

    click('blocking-copy', 0)
    await settle()
    expect(writeText).toHaveBeenCalledWith('UPDATE dbo.orders SET total = 0')
    expect(lastNotice()?.message).toBe('Statement copied.')

    click('blocking-copy', 1)
    await settle()
    expect(writeText).toHaveBeenLastCalledWith('SELECT * FROM dbo.orders')

    click('blocking-copy', 2)
    await settle()
    expect(writeText).toHaveBeenCalledTimes(3)
  })

  it('reports a clipboard that refuses the text or that is missing', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('denied'))
    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    apiStub.blockingSessions.mockResolvedValue(report)
    mountDialog()
    await settle()
    click('blocking-copy')
    await settle()
    expect(lastNotice()?.level).toBe('error')

    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: undefined,
    })
    click('blocking-copy')
    await settle()
    expect(lastNotice()?.level).toBe('warning')
  })
})
