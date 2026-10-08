import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const AutocompleteStatus = (await import('@/components/AutocompleteStatus.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useExplorerStore } = await import('@/stores/explorer')

const options = { maxColumns: 10, ownConnection: true }
const failure = { category: 'database', message: 'Access denied', detail: 'glue:GetTables' }

function snapshot(complete: boolean) {
  return { database: 'sales', relations: [], columnCount: 10, complete }
}

/** The text of every part of the open menu that has the given test mark. */
function texts(mark: string): string[] {
  return [...document.querySelectorAll(`[data-test="${mark}"]`)].map(
    (element) => element.textContent?.trim() ?? '',
  )
}

describe('AutocompleteStatus', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
  })

  it('shows nothing for a tab without a connection', () => {
    const wrapper = mountWithPlugins(AutocompleteStatus, { props: { connectionId: null } })
    expect(wrapper.find('[data-test="autocomplete-status"]').exists()).toBe(false)
  })

  it('shows nothing while autocomplete has every name', async () => {
    const wrapper = mountWithPlugins(AutocompleteStatus, { props: { connectionId: 'c1' } })
    apiStub.schemaSnapshot.mockResolvedValue(snapshot(true))
    await useExplorerStore().readSnapshot('c1', 'sales', options)
    await settle()
    expect(wrapper.find('[data-test="autocomplete-status"]').exists()).toBe(false)
  })

  it('lists each database with its cause when the user opens the marker', async () => {
    const wrapper = mountWithPlugins(AutocompleteStatus, { props: { connectionId: 'c1' } })
    const explorer = useExplorerStore()
    apiStub.schemaSnapshot.mockResolvedValue(snapshot(false))
    await explorer.readSnapshot('c1', 'sales', options)
    apiStub.schemaSnapshot.mockRejectedValue(failure)
    await explorer.readSnapshot('c1', 'default', options)
    await settle()

    const marker = wrapper.find('[data-test="autocomplete-status"]')
    expect(marker.text()).toBe('Autocomplete is limited')
    expect(marker.attributes('aria-label')).toContain('Autocomplete is limited')
    await marker.trigger('click')
    await settle()

    expect(texts('autocomplete-limit-message')).toEqual([
      "Couldn't read the schema of default.",
      'The schema of sales has more than 10 columns, so only part of it is used. ' +
        'Raise the limit in Settings.',
    ])
    expect(texts('autocomplete-limit-detail')).toEqual(['Access denied\nglue:GetTables'])
    const retries = document.querySelectorAll('[data-test="autocomplete-retry"]')
    expect(retries).toHaveLength(2)
    expect(retries[0]!.getAttribute('aria-label')).toBe('Read the schema of default again')
  })

  it('reads the schema again on a try again, and leaves when the read works', async () => {
    const wrapper = mountWithPlugins(AutocompleteStatus, { props: { connectionId: 'c1' } })
    const explorer = useExplorerStore()
    apiStub.schemaSnapshot.mockRejectedValue(failure)
    await explorer.readSnapshot('c1', 'sales', options)
    await settle()
    await wrapper.find('[data-test="autocomplete-status"]').trigger('click')
    await settle()

    let answer: (value: unknown) => void = () => {}
    apiStub.schemaSnapshot.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const retry = document.querySelector<HTMLElement>('[data-test="autocomplete-retry"]')!
    retry.click()
    await settle()
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)
    expect(retry.classList.contains('v-btn--loading')).toBe(true)

    answer(snapshot(true))
    await settle()
    expect(wrapper.find('[data-test="autocomplete-status"]').exists()).toBe(false)
  })

  it('stops the wait of the button when the read fails again', async () => {
    const wrapper = mountWithPlugins(AutocompleteStatus, { props: { connectionId: 'c1' } })
    apiStub.schemaSnapshot.mockRejectedValue(failure)
    await useExplorerStore().readSnapshot('c1', 'sales', options)
    await settle()
    await wrapper.find('[data-test="autocomplete-status"]').trigger('click')
    await settle()

    const retry = document.querySelector<HTMLElement>('[data-test="autocomplete-retry"]')!
    retry.click()
    await settle()
    await settle()
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)
    const after = document.querySelector('[data-test="autocomplete-retry"]')!
    expect(after.classList.contains('v-btn--loading')).toBe(false)
  })
})
