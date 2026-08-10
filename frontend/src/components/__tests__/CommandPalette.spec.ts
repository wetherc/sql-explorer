import { describe, expect, it, vi } from 'vitest'
import CommandPalette from '@/components/CommandPalette.vue'
import { mountWithPlugins, settle } from './mount'
import type { Command } from '@/lib/commands'

const runNewTab = vi.fn()
const runStop = vi.fn()

function commands(): Command[] {
  return [
    { id: 'tab.new', title: 'New tab', group: 'Tabs', key: 'mod+t', run: runNewTab },
    {
      id: 'query.stop',
      title: 'Stop the statement',
      group: 'Query',
      key: null,
      enabled: () => false,
      run: runStop,
    },
  ]
}

async function mountPalette(open = true) {
  const wrapper = mountWithPlugins(CommandPalette, {
    props: { open, commands: commands(), apple: false },
  })
  await settle()
  return wrapper
}

describe('CommandPalette', () => {
  it('lists every command with its key', async () => {
    await mountPalette()
    const items = document.querySelectorAll('[data-test="palette-item"]')
    expect(items).toHaveLength(2)
    expect(document.body.textContent).toContain('Ctrl + T')
  })

  it('keeps the commands that match the filter', async () => {
    const wrapper = await mountPalette()
    const filter = wrapper.findComponent({ name: 'VTextField' })
    await filter.vm.$emit('update:modelValue', 'stop')
    await settle()
    expect(document.querySelectorAll('[data-test="palette-item"]')).toHaveLength(1)
  })

  it('says so when no command matches', async () => {
    const wrapper = await mountPalette()
    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', 'nothing')
    await settle()
    expect(document.querySelector('[data-test="palette-empty"]')).not.toBeNull()
  })

  it('runs the command the user selected and closes', async () => {
    runNewTab.mockReset()
    const wrapper = await mountPalette()
    const item = document.querySelector('[data-test="palette-item"]') as HTMLElement
    item.click()
    await settle()
    expect(runNewTab).toHaveBeenCalled()
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })

  it('refuses a command that cannot run now', async () => {
    runStop.mockReset()
    const wrapper = await mountPalette()
    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', 'stop')
    await settle()
    const item = document.querySelector('[data-test="palette-item"]') as HTMLElement
    item.click()
    await settle()
    expect(runStop).not.toHaveBeenCalled()
  })

  it('moves through the list with the arrow keys and runs with Enter', async () => {
    runNewTab.mockReset()
    runStop.mockReset()
    await mountPalette()
    const field = document.querySelector('[data-test="palette-filter"] input') as HTMLElement

    // The second command cannot run, so Enter on it does nothing.
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(runStop).not.toHaveBeenCalled()

    // The arrow up comes back to the first command, which can run.
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }))
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(runNewTab).toHaveBeenCalled()
  })

  it('moves nowhere when the list is empty', async () => {
    const wrapper = await mountPalette()
    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', 'nothing')
    await settle()
    const field = document.querySelector('[data-test="palette-filter"] input') as HTMLElement
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(document.querySelector('[data-test="palette-empty"]')).not.toBeNull()
  })

  it('starts again from the first command each time it opens', async () => {
    const wrapper = await mountPalette()
    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', 'stop')
    await settle()
    await wrapper.setProps({ open: false })
    await wrapper.setProps({ open: true })
    await settle()
    expect(document.querySelectorAll('[data-test="palette-item"]')).toHaveLength(2)
  })

  it('closes when the overlay reports it', async () => {
    const wrapper = await mountPalette()
    await wrapper.findComponent({ name: 'VDialog' }).vm.$emit('update:modelValue', false)
    expect(wrapper.emitted('update:open')?.[0]).toEqual([false])
  })
})

describe('CommandPalette as a list a reader can follow', () => {
  /** The field of the palette, which the dialog draws away from the wrapper. */
  function field(): HTMLElement {
    return document.querySelector('[data-test="palette-filter"] input') as HTMLElement
  }

  function items(): HTMLElement[] {
    return [...document.querySelectorAll('[data-test="palette-item"]')] as HTMLElement[]
  }

  async function press(key: string): Promise<void> {
    field().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
    await settle()
  }

  it('names the field and the list, and points the field at the chosen row', async () => {
    await mountPalette()

    expect(field().getAttribute('role')).toBe('combobox')
    expect(field().getAttribute('aria-controls')).toBe('palette-listbox')
    expect(document.querySelector('#palette-listbox')?.getAttribute('role')).toBe('listbox')
    expect(items()[0]?.getAttribute('role')).toBe('option')
    expect(items()[0]?.getAttribute('aria-selected')).toBe('true')
    expect(field().getAttribute('aria-activedescendant')).toBe(items()[0]?.id)

    await press('ArrowDown')

    expect(field().getAttribute('aria-activedescendant')).toBe(items()[1]?.id)
    expect(items()[1]?.getAttribute('aria-selected')).toBe('true')
  })

  it('points at no row while no command matches', async () => {
    const wrapper = await mountPalette()

    await wrapper.findComponent({ name: 'VTextField' }).vm.$emit('update:modelValue', 'nothing')
    await settle()

    expect(field().getAttribute('aria-activedescendant')).toBeNull()
    expect(document.querySelector('[data-test="palette-empty"]')?.getAttribute('role')).toBe(
      'presentation',
    )
  })

  it('reaches the first row and the last row', async () => {
    await mountPalette()

    await press('End')
    const rows = items()
    expect(rows[rows.length - 1]?.getAttribute('aria-selected')).toBe('true')

    await press('Home')
    expect(items()[0]?.getAttribute('aria-selected')).toBe('true')
  })

  it('leaves the choice alone at the ends of the list', async () => {
    await mountPalette()

    await press('Home')
    await press('Home')

    expect(items()[0]?.getAttribute('aria-selected')).toBe('true')
  })

  it('brings the chosen row into the part of the list that shows', async () => {
    await mountPalette()
    const scrollIntoView = vi.fn()
    for (const item of items()) {
      item.scrollIntoView = scrollIntoView
    }

    await press('ArrowDown')

    expect(scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' })
  })
})
