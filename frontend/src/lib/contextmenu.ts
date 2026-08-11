/**
 * The menu that the host offers on a right click.
 *
 * A desktop application draws its own menus, and the menu of the host offers
 * nothing that belongs to this one. It stays where the user works with text,
 * because the cut, the copy and the paste of the host belong there, and the
 * editor draws a menu of its own.
 */

/** The places where the menu of the host is worth keeping. */
const TEXT_PLACES = 'input, textarea, [contenteditable="true"], .monaco-editor'

/** True when the menu of the host belongs at the place of this event. */
export function hostMenuBelongs(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(TEXT_PLACES) !== null
}

/**
 * Holds back the menu of the host everywhere but the places above. The caller
 * gets back a function that stops the watch.
 */
export function holdBackHostMenu(root: Pick<Window, 'addEventListener' | 'removeEventListener'>) {
  const onContextMenu = (event: Event): void => {
    if (!hostMenuBelongs(event.target)) {
      event.preventDefault()
    }
  }
  root.addEventListener('contextmenu', onContextMenu)
  return () => root.removeEventListener('contextmenu', onContextMenu)
}
