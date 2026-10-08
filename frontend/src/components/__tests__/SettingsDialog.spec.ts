import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const SettingsDialog = (await import('../SettingsDialog.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useQueryStore } = await import('@/stores/query')
const { useUiStore } = await import('@/stores/ui')

const lastNotice = () => useUiStore().notices.slice(-1)[0]
const usageLine = () => document.querySelector('[data-test="saved-results-usage"]')
const clearButton = () =>
  document.querySelector('[data-test="clear-saved-results"]') as HTMLButtonElement | null

describe('SettingsDialog saved results', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
  })

  it('reads the disk use each time it opens', async () => {
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 3 * 1024 ** 2, count: 2 })
    const wrapper = mountWithPlugins(SettingsDialog, { props: { open: false } })
    await settle()
    expect(apiStub.savedResultsUsage).not.toHaveBeenCalled()

    await wrapper.setProps({ open: true })
    await settle()
    expect(usageLine()?.textContent).toContain('Saved results use 3.0 MB (2 results).')
    expect(clearButton()?.disabled).toBe(false)

    await wrapper.setProps({ open: false })
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 80, count: 1 })
    await wrapper.setProps({ open: true })
    await settle()
    expect(usageLine()?.textContent).toContain('Saved results use 80 B (1 result).')
  })

  it('turns off the button when nothing is saved', async () => {
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 0, count: 0 })
    mountWithPlugins(SettingsDialog, { props: { open: true } })
    await settle()
    expect(usageLine()?.textContent).toContain('No results are saved on this computer.')
    expect(clearButton()?.disabled).toBe(true)
  })

  it('shows no line when the backend gives no use or fails', async () => {
    mountWithPlugins(SettingsDialog, { props: { open: true } })
    await settle()
    expect(usageLine()).toBeNull()

    apiStub.savedResultsUsage.mockRejectedValue(new Error('no state'))
    mountWithPlugins(SettingsDialog, { props: { open: true } })
    await settle()
    expect(usageLine()).toBeNull()
    expect(lastNotice()?.message).toContain('no state')
  })

  it('clears the saved results and forgets them in the tabs', async () => {
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 2048, count: 2 })
    apiStub.clearSavedResults.mockResolvedValue({ bytes: 2048, count: 2 })
    mountWithPlugins(SettingsDialog, { props: { open: true } })
    await settle()
    const forget = vi.spyOn(useQueryStore(), 'forgetSpills')
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 0, count: 0 })
    clearButton()!.click()
    await settle()
    expect(apiStub.clearSavedResults).toHaveBeenCalled()
    expect(forget).toHaveBeenCalled()
    expect(lastNotice()).toMatchObject({
      level: 'success',
      message: 'Cleared 2 results, freed 2.0 KB.',
    })
    expect(usageLine()?.textContent).toContain('No results are saved on this computer.')
  })

  it('reports a clear that fails and keeps the saved results', async () => {
    apiStub.savedResultsUsage.mockResolvedValue({ bytes: 2048, count: 1 })
    apiStub.clearSavedResults.mockRejectedValue(new Error('disk busy'))
    mountWithPlugins(SettingsDialog, { props: { open: true } })
    await settle()
    const forget = vi.spyOn(useQueryStore(), 'forgetSpills')
    clearButton()!.click()
    await settle()
    expect(forget).not.toHaveBeenCalled()
    expect(lastNotice()?.message).toContain('disk busy')
    expect(usageLine()?.textContent).toContain('Saved results use 2.0 KB (1 result).')
    expect(clearButton()?.disabled).toBe(false)
  })
})
