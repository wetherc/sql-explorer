import { afterEach, describe, expect, it } from 'vitest'
import { holdBackHostMenu, hostMenuBelongs } from '@/lib/contextmenu'

afterEach(() => {
  document.body.innerHTML = ''
})

/** Puts one element in the document and gives it back. */
function place(html: string): Element {
  document.body.innerHTML = html
  return document.body.firstElementChild as Element
}

describe('hostMenuBelongs', () => {
  it('keeps the menu of the host where the user works with text', () => {
    expect(hostMenuBelongs(place('<input />'))).toBe(true)
    expect(hostMenuBelongs(place('<textarea></textarea>'))).toBe(true)
    expect(hostMenuBelongs(place('<div contenteditable="true"></div>'))).toBe(true)
  })

  it('keeps it inside the editor, which draws a menu of its own', () => {
    const editor = place('<div class="monaco-editor"><span class="line">SELECT 1</span></div>')
    expect(hostMenuBelongs(editor.querySelector('.line'))).toBe(true)
  })

  it('holds it back anywhere else', () => {
    expect(hostMenuBelongs(place('<div class="grid"></div>'))).toBe(false)
    expect(hostMenuBelongs(place('<button>Run</button>'))).toBe(false)
  })

  it('holds it back for a target that is no element at all', () => {
    expect(hostMenuBelongs(null)).toBe(false)
    expect(hostMenuBelongs(window)).toBe(false)
  })
})

describe('holdBackHostMenu', () => {
  it('holds the menu back away from text, and lets it stand on text', () => {
    const stop = holdBackHostMenu(window)
    document.body.innerHTML = '<div class="grid"></div><input />'
    const grid = document.querySelector('.grid') as Element
    const field = document.querySelector('input') as Element

    const onGrid = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    grid.dispatchEvent(onGrid)
    expect(onGrid.defaultPrevented).toBe(true)

    const onField = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    field.dispatchEvent(onField)
    expect(onField.defaultPrevented).toBe(false)

    stop()
  })

  it('lets the menu of the host come back once the watch stops', () => {
    const stop = holdBackHostMenu(window)
    stop()
    document.body.innerHTML = '<div class="grid"></div>'

    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    ;(document.querySelector('.grid') as Element).dispatchEvent(event)

    expect(event.defaultPrevented).toBe(false)
  })
})
