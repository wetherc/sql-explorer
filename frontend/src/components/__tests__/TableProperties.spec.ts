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

  it('reports a failure and closes itself', async () => {
    apiStub.tableDetails.mockRejectedValue({ kind: 'database', message: 'no', detail: null })
    const wrapper = mountWithPlugins(TableProperties, { props: { open: true, node: node() } })
    await settle()

    expect(useUiStore().notices[0]?.level).toBe('error')
    expect(wrapper.emitted('close')).toHaveLength(1)
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
