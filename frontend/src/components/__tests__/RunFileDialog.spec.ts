import { beforeEach, describe, expect, it } from 'vitest'
import RunFileDialog from '../RunFileDialog.vue'
import { SET_CHOICE_KEY } from '@/lib/runFile'
import { mountWithPlugins, settle } from './mount'

async function mountDialog(props: Record<string, unknown> = {}) {
  const wrapper = mountWithPlugins(RunFileDialog, {
    props: {
      open: true,
      path: '/a/orders.csv',
      format: 'csv',
      several: true,
      rowLimit: 1_000_000,
      ...props,
    },
  })
  await settle()
  return wrapper
}

function click(selector: string): void {
  ;(document.querySelector(selector) as HTMLElement).click()
}

function checked(test: string): boolean {
  return (document.querySelector(`[data-test="${test}"] input`) as HTMLInputElement).checked
}

describe('RunFileDialog', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('selects the choice of the last run when it opens', async () => {
    localStorage.setItem(SET_CHOICE_KEY, 'each')
    const wrapper = await mountDialog({ open: false })
    await wrapper.setProps({ open: true })
    await settle()
    expect(checked('run-file-each')).toBe(true)

    localStorage.setItem(SET_CHOICE_KEY, 'first')
    await wrapper.setProps({ open: false })
    await wrapper.setProps({ open: true })
    await settle()
    expect(checked('run-file-first')).toBe(true)
  })

  it('names the file of the second result for a CSV or JSON file', async () => {
    await mountDialog()
    expect(document.body.textContent).toContain('Saving to /a/orders.csv')
    expect(document.body.textContent).toContain('One file per result set')
    expect(document.querySelector('[data-test="run-file-names"]')).toBeNull()

    click('[data-test="run-file-each"] input')
    await settle()
    expect(document.querySelector('[data-test="run-file-names"]')?.textContent).toContain(
      'orders-2.csv',
    )
  })

  it('offers a sheet for each result of an Excel file', async () => {
    localStorage.setItem(SET_CHOICE_KEY, 'each')
    await mountDialog({ format: 'xlsx', path: '/a/orders.xlsx' })
    expect(document.body.textContent).toContain('One sheet per result set')
    expect(document.querySelector('[data-test="run-file-names"]')).toBeNull()
  })

  it('reports the choice and keeps it for the next run', async () => {
    const wrapper = await mountDialog()
    click('[data-test="run-file-each"] input')
    await settle()
    click('[data-test="run-file-run"]')
    expect(wrapper.emitted('run')).toEqual([[true]])
    expect(localStorage.getItem(SET_CHOICE_KEY)).toBe('each')
  })

  it('asks nothing about the results of a statement with one result', async () => {
    localStorage.setItem(SET_CHOICE_KEY, 'each')
    const wrapper = await mountDialog({ several: false })
    expect(document.querySelector('[data-test="run-file-sets"]')).toBeNull()
    click('[data-test="run-file-run"]')
    expect(wrapper.emitted('run')).toEqual([[false]])
  })

  it('warns when the row limit is past the room of an Excel sheet', async () => {
    const wrapper = await mountDialog({ format: 'xlsx', path: '/a/orders.xlsx' })
    expect(document.querySelector('[data-test="run-file-excel-limit"]')).toBeNull()

    await wrapper.setProps({ rowLimit: 5_000_000 })
    await settle()
    const warning = document.querySelector('[data-test="run-file-excel-limit"]')
    expect(warning?.textContent).toContain('1,048,575 rows')
    expect(warning?.textContent).toContain('5,000,000')
    click('[data-test="run-file-csv"]')
    expect(wrapper.emitted('csv')).toHaveLength(1)
  })

  it('gives no Excel warning for a CSV file', async () => {
    await mountDialog({ rowLimit: 5_000_000 })
    expect(document.querySelector('[data-test="run-file-excel-limit"]')).toBeNull()
  })

  it('reports a cancel from the button and from the overlay', async () => {
    const wrapper = await mountDialog()
    click('[data-test="run-file-cancel"]')
    await wrapper.findComponent({ name: 'AppDialog' }).vm.$emit('update:modelValue', false)
    expect(wrapper.emitted('cancel')).toHaveLength(2)
  })
})
