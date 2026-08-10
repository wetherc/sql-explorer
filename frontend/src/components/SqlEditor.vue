<template>
  <div ref="host" class="sql-editor" data-test="sql-editor"></div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { monaco, registerMonacoThemes } from '@/plugins/monaco'
import { completionsFor, statementAt, wordBefore, type SchemaIndex } from '@/lib/sql'
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
  (event: 'execute', statement: string): void
  (event: 'execute-all'): void
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
  if (!host.value) {
    return
  }
  registerMonacoThemes()
  editor = monaco.editor.create(host.value, {
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

  editor.onDidChangeModelContent(() => {
    if (applyingExternalValue) {
      return
    }
    emit('update:modelValue', editor?.getValue() ?? '')
  })

  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
    emit('execute', currentStatement())
  })
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter, () => {
    emit('execute-all')
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
