import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import SqlEditor from '@/components/SqlEditor.vue'
import { monaco } from '@/plugins/monaco'
import type { editor as MonacoEditor, languages } from 'monaco-editor'
import { emptySchemaIndex } from '@/lib/sql'
import { Dialect } from '@/types/api'

type Handler = () => void

/** Hands the stub to the code under test, which only calls what it needs. */
function asEditor(stub: unknown): MonacoEditor.IStandaloneCodeEditor {
  return stub as MonacoEditor.IStandaloneCodeEditor
}

/** Reads the suggestions of the provider that the editor registered. */
function suggestionsOf(
  stub: ReturnType<typeof stubEditor>,
  column: number,
): languages.CompletionItem[] {
  const provider = vi.mocked(monaco.languages.registerCompletionItemProvider).mock.calls[0]?.[1]
  const result = provider?.provideCompletionItems(
    stub.editor.getModel() as unknown as MonacoEditor.ITextModel,
    { lineNumber: 1, column } as monaco.Position,
    {} as languages.CompletionContext,
    {} as never,
  )
  return (result as languages.CompletionList | undefined)?.suggestions ?? []
}

/** Builds a stand-in for the editor and records what it was asked to do. */
function stubEditor(value = 'SELECT 1;\nSELECT 2') {
  const commands: Record<number, Handler> = {}
  const actions: Record<string, Handler> = {}
  let contentHandler: Handler = () => {}
  const editor = {
    getValue: vi.fn(() => value),
    setValue: vi.fn((next: string) => {
      value = next
      // The real editor reports a write as a change of its model.
      contentHandler()
    }),
    getModel: vi.fn(() => ({
      getValue: () => value,
      // The whole range gives the whole text; any other range stands for
      // the selection of the user.
      getValueInRange: vi.fn((range: { whole?: boolean }) => (range.whole ? value : 'SELECTED')),
      getFullModelRange: vi.fn(() => ({ whole: true })),
      getOffsetAt: vi.fn(() => 0),
      getWordUntilPosition: vi.fn(() => ({ startColumn: 1, endColumn: 4 })),
    })),
    getSelection: vi.fn(() => ({ isEmpty: () => true })),
    getPosition: vi.fn(() => ({ lineNumber: 1, column: 1 })),
    onDidChangeModelContent: vi.fn((handler: Handler) => {
      contentHandler = handler
    }),
    addCommand: vi.fn((key: number, handler: Handler) => {
      commands[key] = handler
    }),
    addAction: vi.fn((action: { id: string; run: Handler }) => {
      actions[action.id] = action.run
      return { dispose: vi.fn() }
    }),
    updateOptions: vi.fn(),
    executeEdits: vi.fn(),
    focus: vi.fn(),
    dispose: vi.fn(),
  }
  return {
    editor,
    commands,
    actions,
    fireContentChange: () => contentHandler(),
    setValue: (next: string) => {
      value = next
    },
  }
}

const RUN_STATEMENT = monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter
const RUN_ALL = monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter
const FORMAT_ACTION = 'sql-explorer.format'

describe('SqlEditor', () => {
  beforeEach(() => {
    vi.mocked(monaco.editor.create).mockReset()
    vi.mocked(monaco.editor.setTheme).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReturnValue({
      dispose: vi.fn(),
    })
  })

  it('builds the editor with the settings it was given', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, {
      props: {
        modelValue: 'SELECT 1',
        fontSize: 18,
        wordWrap: true,
        showLineNumbers: false,
        readOnly: true,
      },
    })
    expect(monaco.editor.create).toHaveBeenCalledWith(
      expect.any(Object),
      expect.objectContaining({
        value: 'SELECT 1',
        language: 'sql',
        fontSize: 18,
        wordWrap: 'on',
        lineNumbers: 'off',
        readOnly: true,
      }),
    )
  })

  it('reports the text the user typed', async () => {
    const stub = stubEditor('SELECT 9')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })
    stub.fireContentChange()
    expect(wrapper.emitted('update:modelValue')).toEqual([['SELECT 9']])
  })

  it('asks to run the statement under the cursor', () => {
    const stub = stubEditor('SELECT 1;\nSELECT 2')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1;\nSELECT 2' } })
    stub.commands[RUN_STATEMENT]?.()
    expect(wrapper.emitted('execute')).toEqual([['SELECT 1']])
  })

  it('asks to run the selection when there is one', () => {
    const stub = stubEditor()
    stub.editor.getSelection.mockReturnValue({ isEmpty: () => false } as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    stub.commands[RUN_STATEMENT]?.()
    expect(wrapper.emitted('execute')).toEqual([['SELECTED']])
  })

  it('asks to run the whole script', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    stub.commands[RUN_ALL]?.()
    expect(wrapper.emitted('execute-all')).toHaveLength(1)
  })

  it('gives the whole text when the editor holds no model', () => {
    const stub = stubEditor()
    stub.editor.getModel.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    stub.commands[RUN_STATEMENT]?.()
    expect(wrapper.emitted('execute')).toEqual([['SELECT 1']])
  })

  it('reads from the start when the cursor is nowhere', () => {
    const stub = stubEditor('SELECT 7')
    stub.editor.getPosition.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 7' } })
    stub.commands[RUN_STATEMENT]?.()
    expect(wrapper.emitted('execute')).toEqual([['SELECT 7']])
  })

  it('writes a new text into the editor and reports nothing back', async () => {
    const stub = stubEditor('old')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'old' } })

    await wrapper.setProps({ modelValue: 'new' })
    expect(stub.editor.setValue).toHaveBeenCalledWith('new')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('leaves the editor alone when the text already matches', async () => {
    const stub = stubEditor('same')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'other' } })
    await wrapper.setProps({ modelValue: 'same' })
    expect(stub.editor.setValue).not.toHaveBeenCalled()
  })

  it('changes the theme and the settings of the editor', async () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })

    await wrapper.setProps({ theme: 'sql-explorer-light' })
    expect(monaco.editor.setTheme).toHaveBeenCalledWith('sql-explorer-light')

    await wrapper.setProps({ fontSize: 20 })
    expect(stub.editor.updateOptions).toHaveBeenCalledWith(
      expect.objectContaining({ fontSize: 20 }),
    )
  })

  it('gives the completions the index holds', () => {
    const stub = stubEditor('sel')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, {
      props: {
        modelValue: 'sel',
        dialect: Dialect.Postgres,
        schemaIndex: {
          ...emptySchemaIndex(),
          tables: [{ name: 'sales', qualifier: 'public' }],
        },
      },
    })

    const suggestions = suggestionsOf(stub, 4)
    expect(suggestions.map((item) => item.label)).toContain('sales')
    expect(suggestions[0]?.kind).toBe(monaco.languages.CompletionItemKind.Struct)
  })

  it('offers only the keywords when no index is given', () => {
    const stub = stubEditor('sel')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, { props: { modelValue: 'sel' } })

    const suggestions = suggestionsOf(stub, 4)
    expect(suggestions.every((item) => item.detail === 'keyword')).toBe(true)
    expect(suggestions[0]?.kind).toBe(monaco.languages.CompletionItemKind.Keyword)
  })

  it('marks each kind of name with its own icon', () => {
    const stub = stubEditor('a')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, {
      props: {
        modelValue: 'a',
        schemaIndex: {
          databases: ['adb'],
          schemas: ['aschema'],
          tables: [{ name: 'atable', qualifier: 'q' }],
          columns: [{ name: 'acolumn', table: 'atable', dataType: 'int' }],
        },
      },
    })
    const kinds = suggestionsOf(stub, 2)
      .slice(0, 4)
      .map((item) => item.kind)
    expect(kinds).toEqual([
      monaco.languages.CompletionItemKind.Field,
      monaco.languages.CompletionItemKind.Struct,
      monaco.languages.CompletionItemKind.Folder,
      monaco.languages.CompletionItemKind.Module,
    ])
  })

  it('offers the actions a parent can call', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1;\nSELECT 2' } })
    const exposed = wrapper.vm as unknown as {
      focus: () => void
      currentStatement: () => string
      insert: (text: string) => void
    }

    exposed.focus()
    expect(stub.editor.focus).toHaveBeenCalled()
    expect(exposed.currentStatement()).toBe('SELECT 1')

    exposed.insert('orders')
    expect(stub.editor.executeEdits).toHaveBeenCalled()
  })

  it('inserts nothing when the editor has no selection', () => {
    const stub = stubEditor()
    stub.editor.getSelection.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })
    ;(wrapper.vm as unknown as { insert: (text: string) => void }).insert('x')
    expect(stub.editor.executeEdits).not.toHaveBeenCalled()
  })

  it('lays out the whole text with the key of the editor', () => {
    const stub = stubEditor('select a,b from t where x=1')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, { props: { modelValue: 'select a,b from t where x=1' } })

    expect(stub.editor.addAction).toHaveBeenCalledWith(
      expect.objectContaining({
        id: FORMAT_ACTION,
        keybindings: [monaco.KeyMod.Shift | monaco.KeyMod.Alt | monaco.KeyCode.KeyF],
      }),
    )

    stub.actions[FORMAT_ACTION]?.()
    expect(stub.editor.executeEdits).toHaveBeenCalledWith('format', [
      expect.objectContaining({ text: 'SELECT\n  a,\n  b\nFROM\n  t\nWHERE\n  x = 1' }),
    ])
  })

  it('lays out the selection alone when there is one', () => {
    const stub = stubEditor('select 1')
    stub.editor.getSelection.mockReturnValue({ isEmpty: () => false } as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, { props: { modelValue: 'select 1' } })

    stub.actions[FORMAT_ACTION]?.()
    expect(stub.editor.executeEdits).toHaveBeenCalledWith('format', [
      expect.objectContaining({ text: 'SELECTED' }),
    ])
  })

  it('reports a text that it cannot lay out', () => {
    const stub = stubEditor('SELECT * FROM (')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT * FROM (' } })

    stub.actions[FORMAT_ACTION]?.()
    expect(stub.editor.executeEdits).not.toHaveBeenCalled()
    expect(wrapper.emitted('format-failed')?.[0]?.[0]).toContain('Parse error')
  })

  it('lays out nothing when the text holds only spaces', () => {
    const stub = stubEditor('   ')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, { props: { modelValue: '   ' } })

    stub.actions[FORMAT_ACTION]?.()
    expect(stub.editor.executeEdits).not.toHaveBeenCalled()
  })

  it('lays out nothing when the editor holds no model', () => {
    const stub = stubEditor('select 1')
    stub.editor.getModel.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'select 1' } })
    ;(wrapper.vm as unknown as { format: () => void }).format()
    expect(stub.editor.executeEdits).not.toHaveBeenCalled()
  })

  it('lays out nothing after the editor is gone', () => {
    const stub = stubEditor('select 1')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'select 1' } })
    wrapper.unmount()
    ;(wrapper.vm as unknown as { format: () => void }).format()
    expect(stub.editor.executeEdits).not.toHaveBeenCalled()
  })

  it('closes the editor when the view goes away', () => {
    const stub = stubEditor()
    const dispose = vi.fn()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReturnValue({ dispose })
    void dispose
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })
    wrapper.unmount()
    expect(stub.editor.dispose).toHaveBeenCalled()
    expect(dispose).toHaveBeenCalled()
  })

  it('gives the text it was given when no editor was built', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    wrapper.unmount()
    expect((wrapper.vm as unknown as { currentStatement: () => string }).currentStatement()).toBe(
      'SELECT 1',
    )
  })
})

describe('SqlEditor settings', () => {
  beforeEach(() => {
    vi.mocked(monaco.editor.create).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReturnValue({
      dispose: vi.fn(),
    })
  })

  it('passes every setting on to the editor when one changes', async () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, {
      props: { modelValue: '', wordWrap: false, showLineNumbers: true },
    })

    await wrapper.setProps({ wordWrap: true, showLineNumbers: false, readOnly: true })
    expect(stub.editor.updateOptions).toHaveBeenCalledWith({
      fontSize: 13,
      wordWrap: 'on',
      lineNumbers: 'off',
      readOnly: true,
    })
  })

  it('builds the editor with wrapping off and no line numbers', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    mount(SqlEditor, { props: { modelValue: '', wordWrap: false, showLineNumbers: false } })
    expect(monaco.editor.create).toHaveBeenCalledWith(
      expect.any(Object),
      expect.objectContaining({ wordWrap: 'off', lineNumbers: 'off' }),
    )
  })
})
