<template>
  <div ref="host" class="sql-editor" data-test="sql-editor"></div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { monaco, registerMonacoThemes, registerSqlParameters } from '@/plugins/monaco'
import { clearCompletionSource, installSqlCompletions, setCompletionSource } from '@/lib/completion'
import { emptySchemaIndex, formatSql, statementRunAt, type SchemaIndex } from '@/lib/sql'
import { Dialect } from '@/types/api'
import { keepViewState, modelFor } from './editorModels'

/** A place in the text where the last run failed, with the reason. */
export interface ErrorMarker {
  line: number
  column: number
  message: string
}

/** The owner of the markers this editor sets on its model. */
const MARKER_OWNER = 'sql-explorer'

const props = withDefaults(
  defineProps<{
    modelValue: string
    theme?: string
    fontSize?: number
    wordWrap?: boolean
    showLineNumbers?: boolean
    schemaIndex?: SchemaIndex
    dialect?: Dialect
    readOnly?: boolean
    /**
     * The tab this editor belongs to. An editor with a key keeps its model
     * and view state after unmount, so undo and the cursor come back.
     */
    modelKey?: string
    /** The place of the last failure, which the editor marks. */
    errorMarker?: ErrorMarker | null
  }>(),
  {
    theme: 'sql-explorer-dark',
    fontSize: 13,
    wordWrap: false,
    showLineNumbers: true,
    schemaIndex: undefined,
    dialect: Dialect.MsSql,
    readOnly: false,
    modelKey: undefined,
    errorMarker: null,
  },
)

const emit = defineEmits<{
  (event: 'update:modelValue', value: string): void
  (event: 'format-failed', message: string): void
  (event: 'show-keys'): void
  (event: 'run-statement'): void
  (event: 'run-all'): void
  /** The user changed the text while the mark of a failure was shown. */
  (event: 'marker-cleared'): void
}>()

const host = ref<HTMLElement | null>(null)
let editor: monaco.editor.IStandaloneCodeEditor | null = null
/** The address of the model of this editor, which keys its own names. */
let modelUri: string | null = null
/** True while the model shows the mark of a failure. */
let markerShown = false
/** True while the editor writes into the model itself. */
let applyingExternalValue = false
/**
 * The text that the editor last sent to the parent, or `null` after a write
 * from outside. The model has this text until the next change.
 */
let emittedValue: string | null = null

/**
 * Returns the text to run: the selection when there is one, and otherwise
 * the statement that holds the cursor.
 */
function currentStatement(): string {
  return currentRun().text
}

/**
 * The text to run and the place in the editor where it begins, so the store
 * can move the place of a failure into the coordinates of the editor. The
 * place is missing when the script has no statement, and when a MySQL
 * statement under a DELIMITER command starts inside a line.
 */
function currentRun(): { text: string; start?: { line: number; column: number } } {
  const model = editor?.getModel()
  if (!editor || !model) {
    return { text: props.modelValue }
  }
  const selection = editor.getSelection()
  if (selection && !selection.isEmpty()) {
    return {
      text: model.getValueInRange(selection),
      start: { line: selection.startLineNumber, column: selection.startColumn },
    }
  }
  const position = editor.getPosition()
  const offset = position ? model.getOffsetAt(position) : 0
  const { text, start, wrapped } = statementRunAt(model.getValue(), offset, props.dialect)
  if (text === '') {
    return { text }
  }
  const at = model.getPositionAt(start)
  if (!wrapped) {
    return { text, start: { line: at.lineNumber, column: at.column } }
  }
  // The sent text has a DELIMITER line in front of the statement. The line
  // above the statement stands for it, so the lines that follow match. A
  // statement that starts inside a line gets no place, because its columns
  // would not match.
  return at.column === 1 ? { text, start: { line: at.lineNumber - 1, column: 1 } } : { text }
}

/**
 * Lays out the selection, or the whole text when nothing is selected. The
 * write goes through `executeEdits`, so one undo step takes it back.
 *
 * The action below carries the key Shift+Alt+F. The editor holds that key
 * for its own format action, and an action of this editor takes it over, so
 * the key, the context menu and the toolbar button all reach this function.
 */
function formatText(): void {
  const model = editor?.getModel()
  if (!editor || !model) {
    return
  }
  const selection = editor.getSelection()
  const range = selection && !selection.isEmpty() ? selection : model.getFullModelRange()
  const source = model.getValueInRange(range)
  if (source.trim() === '') {
    return
  }
  try {
    editor.executeEdits('format', [
      { range, text: formatSql(source, props.dialect), forceMoveMarkers: true },
    ])
  } catch (error) {
    emit('format-failed', error instanceof Error ? error.message : String(error))
  }
}

/**
 * Tells the provider of the application which names this editor offers. The
 * provider itself is registered once for the SQL language, so five open tabs
 * still give one list of each name.
 */
function registerCompletions(): void {
  const uri = editor?.getModel()?.uri?.toString()
  if (!uri) {
    return
  }
  modelUri = uri
  setCompletionSource(modelUri, () => ({
    index: props.schemaIndex ?? emptySchemaIndex(),
    dialect: props.dialect,
  }))
  installSqlCompletions()
}

/**
 * Puts a text that came from outside the editor, such as an opened file or an
 * entry of the history, into the model. `setValue` clears the undo stack, so
 * the write replaces the whole range as one edit, and one undo step gives the
 * text before it back. The model takes the edit also in a read-only editor,
 * where `executeEdits` refuses it. The cursor then goes to the start, as it
 * does after `setValue`.
 */
function applyExternalValue(instance: monaco.editor.IStandaloneCodeEditor, value: string): void {
  const model = instance.getModel()
  if (!model || model.getValue() === value) {
    return
  }
  applyingExternalValue = true
  emittedValue = null
  model.pushStackElement()
  model.pushEditOperations(
    instance.getSelections(),
    [{ range: model.getFullModelRange(), text: value }],
    () => null,
  )
  model.pushStackElement()
  applyingExternalValue = false
  instance.setPosition({ lineNumber: 1, column: 1 })
  instance.setScrollTop(0)
}

onMounted(() => {
  registerMonacoThemes()
  registerSqlParameters()
  // The template above always draws the host element, so it is present by
  // the time this runs.
  const element = host.value as HTMLElement
  const keptEntry = props.modelKey ? modelFor(props.modelKey, props.modelValue) : null
  const instance = monaco.editor.create(element, {
    ...(keptEntry ? { model: keptEntry.model } : { value: props.modelValue, language: 'sql' }),
    theme: props.theme,
    fontSize: props.fontSize,
    wordWrap: props.wordWrap ? 'on' : 'off',
    lineNumbers: props.showLineNumbers ? 'on' : 'off',
    readOnly: props.readOnly,
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    renderLineHighlight: 'all',
    tabSize: 2,
    padding: { top: 8, bottom: 8 },
    // The names of the schema and the keywords are the whole list, so the
    // words of the document add nothing but noise.
    wordBasedSuggestions: 'off',
  })
  editor = instance
  if (keptEntry) {
    if (keptEntry.viewState) {
      instance.restoreViewState(keptEntry.viewState)
    }
    // The store can change the text while the view is away, for example
    // when the user opens a file into the tab.
    applyExternalValue(instance, props.modelValue)
  }

  instance.onDidChangeModelContent(() => {
    // The editor also reports the writes this component makes itself, and
    // those must not travel back out as a change by the user.
    if (applyingExternalValue) {
      return
    }
    // The place of an old failure means nothing after an edit.
    if (markerShown) {
      setMarker(null)
      emit('marker-cleared')
    }
    emittedValue = instance.getValue()
    emit('update:modelValue', emittedValue)
  })

  // The shell binds the keys of the application on the window. Monaco stops
  // each key that it binds itself, so the window does not see that key. The
  // editor therefore binds the keys that Monaco holds for something else, so
  // that they reach the command of this application.
  instance.addAction({
    id: 'sql-explorer.run',
    label: 'Run statement',
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
    run: () => emit('run-statement'),
  })

  instance.addAction({
    id: 'sql-explorer.runAll',
    label: 'Run script',
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
    run: () => emit('run-all'),
  })

  instance.addAction({
    id: 'sql-explorer.keys',
    label: 'Keyboard shortcuts',
    keybindings: [monaco.KeyCode.F1],
    run: () => emit('show-keys'),
  })

  instance.addAction({
    id: 'sql-explorer.format',
    label: 'Format SQL',
    keybindings: [monaco.KeyMod.Shift | monaco.KeyMod.Alt | monaco.KeyCode.KeyF],
    contextMenuGroupId: 'modification',
    run: () => formatText(),
  })

  // The watch stands inside the hook, so it reads the editor of this mount,
  // and Vue stops it at unmount.
  watch(
    () => props.modelValue,
    (value) => {
      // The text that the editor itself just sent comes back through the
      // parent. The model already has it, so the text of the model is not
      // read again and compared.
      if (value !== emittedValue) {
        applyExternalValue(instance, value)
      }
    },
  )

  registerCompletions()

  watch(
    () => props.errorMarker,
    (marker) => setMarker(marker),
    { immediate: true },
  )
})

/** Draws the mark of a failure on the model, or clears it. */
function setMarker(marker: ErrorMarker | null): void {
  const model = editor?.getModel()
  if (!model) {
    return
  }
  // The text can change after the run, so a place past the end of the text
  // gets no mark, and a column past the end of its line moves to that end.
  const shown = marker !== null && marker.line <= model.getLineCount()
  const markers = shown
    ? [
        {
          severity: monaco.MarkerSeverity.Error,
          message: marker.message,
          startLineNumber: marker.line,
          startColumn: Math.min(marker.column, model.getLineMaxColumn(marker.line)),
          endLineNumber: marker.line,
          endColumn: model.getLineMaxColumn(marker.line),
        },
      ]
    : []
  monaco.editor.setModelMarkers(model, MARKER_OWNER, markers)
  markerShown = shown
}

/** Moves the cursor to a place in the text and shows it. */
function reveal(line: number, column: number): void {
  if (!editor) {
    return
  }
  editor.setPosition({ lineNumber: line, column })
  editor.revealLineInCenter(line)
  editor.focus()
}

watch(
  () => props.theme,
  (theme) => monaco.editor.setTheme(theme),
)

watch(
  () => [props.fontSize, props.wordWrap, props.showLineNumbers, props.readOnly],
  () => {
    editor?.updateOptions({
      fontSize: props.fontSize,
      wordWrap: props.wordWrap ? 'on' : 'off',
      lineNumbers: props.showLineNumbers ? 'on' : 'off',
      readOnly: props.readOnly,
    })
  },
)

onBeforeUnmount(() => {
  if (modelUri) {
    clearCompletionSource(modelUri)
    modelUri = null
  }
  if (props.modelKey && editor) {
    keepViewState(props.modelKey, editor.saveViewState())
  }
  // An editor does not dispose a model it was given, so a kept model stays.
  editor?.dispose()
  editor = null
})

defineExpose({
  focus: () => editor?.focus(),
  currentStatement,
  currentRun,
  format: formatText,
  reveal,
  insert: (text: string) => {
    const selection = editor?.getSelection()
    if (editor && selection) {
      editor.executeEdits('insert', [{ range: selection, text, forceMoveMarkers: true }])
      editor.focus()
    }
  },
})
</script>

<style scoped>
.sql-editor {
  width: 100%;
  height: 100%;
  min-height: 0;
}
</style>
