<template>
  <div ref="host" class="sql-editor" data-test="sql-editor"></div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { monaco, registerMonacoThemes } from '@/plugins/monaco'
import { completionsFor, formatSql, statementAt, wordBefore, type SchemaIndex } from '@/lib/sql'
import { Dialect } from '@/types/api'

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
  }>(),
  {
    theme: 'sql-explorer-dark',
    fontSize: 13,
    wordWrap: false,
    showLineNumbers: true,
    schemaIndex: undefined,
    dialect: Dialect.MsSql,
    readOnly: false,
  },
)

const emit = defineEmits<{
  (event: 'update:modelValue', value: string): void
  (event: 'format-failed', message: string): void
  (event: 'show-keys'): void
}>()

const host = ref<HTMLElement | null>(null)
let editor: monaco.editor.IStandaloneCodeEditor | null = null
let completionProvider: monaco.IDisposable | null = null
/** True while the editor writes into the model itself. */
let applyingExternalValue = false

/**
 * Returns the text to run: the selection when there is one, and otherwise
 * the statement that holds the cursor.
 */
function currentStatement(): string {
  if (!editor) {
    return props.modelValue
  }
  const model = editor.getModel()
  const selection = editor.getSelection()
  if (model && selection && !selection.isEmpty()) {
    return model.getValueInRange(selection)
  }
  if (!model) {
    return props.modelValue
  }
  const position = editor.getPosition()
  const offset = position ? model.getOffsetAt(position) : 0
  return statementAt(model.getValue(), offset)
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

function registerCompletions() {
  completionProvider?.dispose()
  completionProvider = monaco.languages.registerCompletionItemProvider('sql', {
    provideCompletionItems: (model: monaco.editor.ITextModel, position: monaco.Position) => {
      const offset = model.getOffsetAt(position)
      const prefix = wordBefore(model.getValue(), offset)
      const word = model.getWordUntilPosition(position)
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }
      const index: SchemaIndex = props.schemaIndex ?? {
        databases: [],
        schemas: [],
        tables: [],
        columns: [],
      }
      const kinds = monaco.languages.CompletionItemKind
      const kindFor = (kind: string): monaco.languages.CompletionItemKind => {
        switch (kind) {
          case 'database':
            return kinds.Module
          case 'schema':
            return kinds.Folder
          case 'table':
            return kinds.Struct
          case 'column':
            return kinds.Field
          default:
            return kinds.Keyword
        }
      }
      return {
        suggestions: completionsFor(prefix, index, props.dialect).map((item) => ({
          label: item.label,
          detail: item.detail,
          insertText: item.insertText,
          kind: kindFor(item.kind),
          range,
        })),
      }
    },
  })
}

onMounted(() => {
  registerMonacoThemes()
  // The template above always draws the host element, so it is present by
  // the time this runs.
  const element = host.value as HTMLElement
  const instance = monaco.editor.create(element, {
    value: props.modelValue,
    language: 'sql',
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
  })
  editor = instance

  instance.onDidChangeModelContent(() => {
    // The editor also reports the writes this component makes itself, and
    // those must not travel back out as a change by the user.
    if (applyingExternalValue) {
      return
    }
    emit('update:modelValue', instance.getValue())
  })

  // The keys of the application are bound once by the shell. The editor
  // therefore binds only the two keys that the editor itself already holds
  // for something else, so that they reach the command of this application.
  instance.addAction({
    id: 'sql-explorer.keys',
    label: 'Show the key list',
    keybindings: [monaco.KeyCode.F1],
    run: () => emit('show-keys'),
  })

  instance.addAction({
    id: 'sql-explorer.format',
    label: 'Format the statement',
    keybindings: [monaco.KeyMod.Shift | monaco.KeyMod.Alt | monaco.KeyCode.KeyF],
    contextMenuGroupId: 'modification',
    run: () => formatText(),
  })

  registerCompletions()
})

watch(
  () => props.modelValue,
  (value) => {
    if (editor && editor.getValue() !== value) {
      applyingExternalValue = true
      editor.setValue(value)
      applyingExternalValue = false
    }
  },
)

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
  completionProvider?.dispose()
  completionProvider = null
  editor?.dispose()
  editor = null
})

defineExpose({
  focus: () => editor?.focus(),
  currentStatement,
  format: formatText,
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
