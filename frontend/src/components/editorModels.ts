import { monaco } from '@/plugins/monaco'

/**
 * The text model and the view state (cursor, selection, scroll) of one tab.
 * A tab keeps its model while its view is unmounted, so the undo steps and
 * the place of the cursor come back when the tab mounts again.
 */
interface KeptEditor {
  model: monaco.editor.ITextModel
  viewState: monaco.editor.ICodeEditorViewState | null
}

const kept = new Map<string, KeptEditor>()

/** Gives the model of a tab, and makes one with the text when there is none. */
export function modelFor(key: string, value: string): KeptEditor {
  let entry = kept.get(key)
  if (!entry) {
    entry = { model: monaco.editor.createModel(value, 'sql'), viewState: null }
    kept.set(key, entry)
  }
  return entry
}

/** Records the view state of a tab when its editor goes away. */
export function keepViewState(
  key: string,
  viewState: monaco.editor.ICodeEditorViewState | null,
): void {
  const entry = kept.get(key)
  if (entry) {
    entry.viewState = viewState
  }
}

/** Frees the model of a tab that closed. */
export function disposeModel(key: string): void {
  kept.get(key)?.model.dispose()
  kept.delete(key)
}

/** The tabs that keep a model, for the clean-up after a close. */
export function keptKeys(): string[] {
  return [...kept.keys()]
}
