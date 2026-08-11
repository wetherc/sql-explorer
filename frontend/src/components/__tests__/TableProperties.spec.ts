import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const TableProperties = (await import('@/components/TableProperties.vue')).default

/** The dialog draws into the overlay of the page, so the tests read that. */
function overlayText(): string {
  return document.body.textContent ?? ''
}

function overlayAll(test: string): Element[] {
  return [...document.querySelectorAll(`[data-test="${test}"]`)]
}
const { mountWithPlugins, settle } = await import('./mount')
const { useUiStore } = await import('@/stores/ui')
const { TableKind } = await import('@/types/api')

function node(overrides: Record<string, unknown> = {}) {
  return {
    key: 'c1/Sales/dbo/table/orders',
    label: 'orders',
    kind: 'table' as const,
    icon: 'mdi-table',
    loading: false,
    loaded: false,
    connectionId: 'c1',
    database: 'Sales',
    schema: 'dbo',
    table: 'orders',
    ...overrides,
  }
}

const details = {
  facts: [{ name: 'Rows', value: '3' }],
  columns: [
    { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    { name: 'note', dataType: 'text', nullable: true, isPrimaryKey: false },
  ],
  indexes: [
    { name: 'pk_orders', columns: ['id'], unique: true, primary: true },
    { name: 'by_note', columns: ['note'], unique: true, primary: false },
    { name: 'by_all', columns: ['id', 'note'], unique: false, primary: false },
  ],
  constraints: [{ name: 'pk_orders', kind: 'primaryKey' as const, columns: ['id'], detail: null }],
}

describe('TableProperties', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    void TableKind
  })

  it('reads the parts of one relation and shows each of them', async () => {
    apiStub.tableDetails.mockResolvedValue(details)
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()

    expect(apiStub.tableDetails).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(overlayText()).toContain('Sales.dbo.orders')
    expect(overlayAll('property-fact')).toHaveLength(1)
    expect(overlayAll('property-column')).toHaveLength(2)
    const indexes = overlayAll('property-index')
    expect(indexes[0]?.textContent).toContain('primary key')
    expect(indexes[1]?.textContent).toContain('unique')
    expect(indexes[2]?.textContent).toContain('id, note')
    expect(overlayAll('property-constraint')).toHaveLength(1)
    void wrapper
  })

  it('reports that an engine holds none of the parts', async () => {
    apiStub.tableDetails.mockResolvedValue({
      facts: [],
      columns: [],
      indexes: [],
      constraints: [],
    })
    const wrapper = mountWithPlugins(TableProperties, {
      props: {
        open: true,
        node: node({ database: undefined, schema: undefined, table: undefined }),
      },
    })
    await settle()

    expect(apiStub.tableDetails).toHaveBeenCalledWith('c1', '', null, 'orders')
    expect(overlayAll('no-facts')).toHaveLength(1)
    expect(overlayAll('no-indexes')).toHaveLength(1)
    expect(overlayAll('no-constraints')).toHaveLength(1)
    expect(overlayText()).toContain('orders')
    void wrapper
  })

  it('reads nothing while it stands closed, and nothing without a relation', async () => {
    const wrapper = mountWithPlugins(TableProperties, { props: { open: false, node: node() } })
    await settle()
    expect(apiStub.tableDetails).not.toHaveBeenCalled()

    const empty = mountWithPlugins(TableProperties, { props: { open: true, node: null } })
    await settle()
    expect(apiStub.tableDetails).not.toHaveBeenCalled()
    expect(overlayText()).toContain('Properties')
    void empty
    void wrapper
  })

  it('holds a failure of the read and stays open', async () => {
    apiStub.tableDetails.mockRejectedValue({
      kind: 'database',
      message: 'The relation is gone.',
      detail: 'relation "orders" does not exist',
    })
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()

    const alert = document.querySelector('[data-test="properties-error"]')
    expect(alert?.textContent).toContain('The relation is gone.')
    expect(alert?.textContent).toContain('relation "orders" does not exist')
    // The dialog is what the user asked for, so it stays.
    expect(wrapper.emitted('close')).toBeUndefined()
    expect(useUiStore().notices).toHaveLength(0)
  })

  it('reads again on request, and shows what it found', async () => {
    apiStub.tableDetails.mockRejectedValueOnce({
      kind: 'database',
      message: 'The relation is gone.',
      detail: null,
    })
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()
    expect(document.querySelector('[data-test="properties-error"]')).not.toBeNull()

    apiStub.tableDetails.mockResolvedValue(details)
    const retry = document.querySelector('[data-test="properties-retry"]') as HTMLElement
    retry.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(document.querySelector('[data-test="properties-error"]')).toBeNull()
    expect(document.querySelectorAll('[data-test="property-column"]').length).toBeGreaterThan(0)
    void wrapper
  })

  it('offers no way to read again while nothing failed', async () => {
    apiStub.tableDetails.mockResolvedValue(details)
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()

    expect(document.querySelector('[data-test="properties-retry"]')).toBeNull()
    void wrapper
  })

  it('closes itself when the overlay reports it', async () => {
    apiStub.tableDetails.mockResolvedValue(details)
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()
    await wrapper.findComponent({ name: 'VDialog' }).vm.$emit('update:modelValue', false)
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('closes itself from its own button', async () => {
    apiStub.tableDetails.mockResolvedValue(details)
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()
    const close = document.querySelector('[data-test="properties-close"]') as HTMLElement
    close.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(wrapper.emitted('close')).toHaveLength(1)
  })
})
