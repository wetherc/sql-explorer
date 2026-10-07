import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import SqlEditor from '@/components/SqlEditor.vue'
import { monaco } from '@/plugins/monaco'
import type { editor as MonacoEditor, languages } from 'monaco-editor'
import { disposeSqlCompletions } from '@/lib/completion'
import { emptySchemaIndex } from '@/lib/sql'
import { Dialect } from '@/types/api'
import { disposeModel, keptKeys } from '@/components/editorModels'

type Handler = () => void

/** Reads the statement that the editor reports to its parent. */
function statementOf(wrapper: { vm: unknown }): string {
  return (wrapper.vm as { currentStatement: () => string }).currentStatement()
}

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
  const actions: Record<string, Handler> = {}
  let contentHandler: Handler = () => {}
  const model = {
    uri: { toString: () => 'model:test' },
    getValue: () => value,
    // The whole range gives the whole text; any other range stands for
    // the selection of the user.
    getValueInRange: vi.fn((range: { whole?: boolean }) => (range.whole ? value : 'SELECTED')),
    getFullModelRange: vi.fn(() => ({ whole: true })),
    getOffsetAt: vi.fn(() => 0),
    getPositionAt: vi.fn((offset: number) => {
      const lines = value.slice(0, offset).split('\n')
      return { lineNumber: lines.length, column: lines[lines.length - 1]!.length + 1 }
    }),
    getLineCount: vi.fn(() => value.split('\n').length),
    getWordUntilPosition: vi.fn(() => ({ startColumn: 1, endColumn: 4 })),
    getLineMaxColumn: vi.fn(() => 12),
    pushStackElement: vi.fn(),
    pushEditOperations: vi.fn(
      (_before: unknown, edits: Array<{ text: string }>, _cursor: () => unknown) => {
        value = edits[0]!.text
        // The real model reports an edit as a change of its content.
        contentHandler()
        return null
      },
    ),
  }
  const editor = {
    getValue: vi.fn(() => value),
    setValue: vi.fn(),
    getModel: vi.fn(() => model),
    getSelection: vi.fn(() => ({ isEmpty: () => true })),
    getSelections: vi.fn(() => [{ cursor: 'user' }]),
    setPosition: vi.fn(),
    setScrollTop: vi.fn(),
    getPosition: vi.fn(() => ({ lineNumber: 1, column: 1 })),
    onDidChangeModelContent: vi.fn((handler: Handler) => {
      contentHandler = handler
    }),
    addAction: vi.fn((action: { id: string; run: Handler }) => {
      actions[action.id] = action.run
      return { dispose: vi.fn() }
    }),
    updateOptions: vi.fn(),
    executeEdits: vi.fn(),
    focus: vi.fn(),
    dispose: vi.fn(),
    saveViewState: vi.fn(() => ({ cursor: 'kept' })),
    restoreViewState: vi.fn(),
    revealLineInCenter: vi.fn(),
  }
  return {
    editor,
    model,
    actions,
    fireContentChange: () => contentHandler(),
    setValue: (next: string) => {
      value = next
    },
  }
}

const FORMAT_ACTION = 'sql-explorer.format'
const KEYS_ACTION = 'sql-explorer.keys'
const RUN_ACTION = 'sql-explorer.run'
const RUN_ALL_ACTION = 'sql-explorer.runAll'

describe('SqlEditor', () => {
  beforeEach(() => {
    vi.mocked(monaco.editor.create).mockReset()
    vi.mocked(monaco.editor.setTheme).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReturnValue({
      dispose: vi.fn(),
    })
    // The provider belongs to the language and not to one editor, so each
    // test starts without one.
    disposeSqlCompletions()
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

  it('gives the statement under the cursor', () => {
    const stub = stubEditor('SELECT 1;\nSELECT 2')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, {
      props: { modelValue: 'SELECT 1;\nSELECT 2', dialect: Dialect.Postgres },
    })
    expect(statementOf(wrapper)).toBe('SELECT 1')
    // MS SQL Server runs the whole batch.
    const mssql = mount(SqlEditor, { props: { modelValue: 'SELECT 1;\nSELECT 2' } })
    expect(statementOf(mssql)).toBe('SELECT 1;\nSELECT 2')
  })

  it('gives the selection when there is one', () => {
    const stub = stubEditor()
    stub.editor.getSelection.mockReturnValue({ isEmpty: () => false } as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    expect(statementOf(wrapper)).toBe('SELECTED')
  })

  it('gives the whole text when the editor has no model', () => {
    const stub = stubEditor()
    stub.editor.getModel.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 1' } })
    expect(statementOf(wrapper)).toBe('SELECT 1')
  })

  it('reads from the start when the cursor is nowhere', () => {
    const stub = stubEditor('SELECT 7')
    stub.editor.getPosition.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'SELECT 7' } })
    expect(statementOf(wrapper)).toBe('SELECT 7')
  })

  it('asks for the key list, which the editor otherwise keeps for itself', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })

    expect(stub.editor.addAction).toHaveBeenCalledWith(
      expect.objectContaining({ id: KEYS_ACTION, keybindings: [monaco.KeyCode.F1] }),
    )
    stub.actions[KEYS_ACTION]?.()
    expect(wrapper.emitted('show-keys')).toHaveLength(1)
  })

  it('binds the run keys, which the editor otherwise keeps for itself', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })

    expect(stub.editor.addAction).toHaveBeenCalledWith(
      expect.objectContaining({
        id: RUN_ACTION,
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
      }),
    )
    expect(stub.editor.addAction).toHaveBeenCalledWith(
      expect.objectContaining({
        id: RUN_ALL_ACTION,
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
      }),
    )
    stub.actions[RUN_ACTION]?.()
    stub.actions[RUN_ALL_ACTION]?.()
    expect(wrapper.emitted('run-statement')).toHaveLength(1)
    expect(wrapper.emitted('run-all')).toHaveLength(1)
  })

  it('writes a new text into the editor and reports nothing back', async () => {
    const stub = stubEditor('old')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'old' } })

    await wrapper.setProps({ modelValue: 'new' })
    expect(stub.editor.getValue()).toBe('new')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('writes a new text as one edit that undo takes back', async () => {
    const stub = stubEditor('old')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'old', readOnly: true } })
    await wrapper.setProps({ modelValue: 'new' })

    // `setValue` clears the undo stack, so the editor does not call it.
    expect(stub.editor.setValue).not.toHaveBeenCalled()
    const model = stub.model
    expect(model.pushEditOperations).toHaveBeenCalledWith(
      [{ cursor: 'user' }],
      [{ range: { whole: true }, text: 'new' }],
      expect.any(Function),
    )
    expect(model.pushEditOperations.mock.calls[0]![2]()).toBeNull()
    // A stop of the undo stack stands on each side of the edit, so the
    // edit is a step of its own.
    expect(model.pushStackElement).toHaveBeenCalledTimes(2)
    expect(model.pushStackElement.mock.invocationCallOrder[0]).toBeLessThan(
      model.pushEditOperations.mock.invocationCallOrder[0]!,
    )
    expect(model.pushStackElement.mock.invocationCallOrder[1]).toBeGreaterThan(
      model.pushEditOperations.mock.invocationCallOrder[0]!,
    )
    expect(stub.editor.setPosition).toHaveBeenCalledWith({ lineNumber: 1, column: 1 })
    expect(stub.editor.setScrollTop).toHaveBeenCalledWith(0)
  })

  it('writes nothing when the editor has no model', async () => {
    const stub = stubEditor('old')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'old' } })
    stub.editor.getModel.mockReturnValue(null as never)
    await wrapper.setProps({ modelValue: 'new' })
    expect(stub.model.pushEditOperations).not.toHaveBeenCalled()
  })

  it('leaves the editor alone when the text already matches', async () => {
    const stub = stubEditor('same')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: 'other' } })
    await wrapper.setProps({ modelValue: 'same' })
    expect(stub.model.pushEditOperations).not.toHaveBeenCalled()
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

  it('tells the provider of the application which names it offers', () => {
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

  it('registers the provider once, however many editors open', () => {
    const first = stubEditor('a')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(first.editor))
    mount(SqlEditor, { props: { modelValue: 'a' } })
    mount(SqlEditor, { props: { modelValue: 'a' } })
    expect(vi.mocked(monaco.languages.registerCompletionItemProvider)).toHaveBeenCalledTimes(1)
  })

  it('offers nothing of its own once the editor is gone', () => {
    const stub = stubEditor('sel')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, {
      props: {
        modelValue: 'sel',
        schemaIndex: { ...emptySchemaIndex(), tables: [{ name: 'sales', qualifier: 'public' }] },
      },
    })
    wrapper.unmount()
    expect(suggestionsOf(stub, 4).map((item) => item.label)).not.toContain('sales')
  })

  it('offers the actions a parent can call', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, {
      props: { modelValue: 'SELECT 1;\nSELECT 2', dialect: Dialect.Postgres },
    })
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

  it('lays out nothing when the editor has no model', () => {
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
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })
    wrapper.unmount()
    expect(stub.editor.dispose).toHaveBeenCalled()
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
    // The provider belongs to the language and not to one editor, so each
    // test starts without one.
    disposeSqlCompletions()
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

  it('keeps the model and the view state of a tab across an unmount', async () => {
    const stub = stubEditor('SELECT 1')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    vi.mocked(monaco.editor.createModel).mockClear()
    const first = mount(SqlEditor, { props: { modelValue: 'SELECT 1', modelKey: 'tab-a' } })
    expect(monaco.editor.createModel).toHaveBeenCalledTimes(1)
    const model = vi.mocked(monaco.editor.createModel).mock.results[0]!.value
    expect(monaco.editor.create).toHaveBeenLastCalledWith(
      expect.any(Object),
      expect.objectContaining({ model }),
    )
    // The first mount has no view state to give back.
    expect(stub.editor.restoreViewState).not.toHaveBeenCalled()
    first.unmount()
    expect(stub.editor.saveViewState).toHaveBeenCalled()

    mount(SqlEditor, { props: { modelValue: 'SELECT 1', modelKey: 'tab-a' } })
    expect(monaco.editor.createModel).toHaveBeenCalledTimes(1)
    expect(stub.editor.restoreViewState).toHaveBeenCalledWith({ cursor: 'kept' })
    expect(keptKeys()).toContain('tab-a')

    disposeModel('tab-a')
    expect(model.dispose).toHaveBeenCalled()
    expect(keptKeys()).not.toContain('tab-a')
    // A second dispose of the same tab does nothing.
    disposeModel('tab-a')
  })

  it('marks the place of a failure and clears the mark on an edit', async () => {
    const stub = stubEditor('SELECT x')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    vi.mocked(monaco.editor.setModelMarkers).mockClear()
    const wrapper = mount(SqlEditor, {
      props: { modelValue: 'SELECT x', errorMarker: { line: 1, column: 8, message: 'no x' } },
    })
    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(stub.model, 'sql-explorer', [
      expect.objectContaining({
        message: 'no x',
        startLineNumber: 1,
        startColumn: 8,
        endLineNumber: 1,
        endColumn: 12,
      }),
    ])

    stub.fireContentChange()
    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(stub.model, 'sql-explorer', [])
    // A second edit has no mark to clear.
    vi.mocked(monaco.editor.setModelMarkers).mockClear()
    stub.fireContentChange()
    expect(monaco.editor.setModelMarkers).not.toHaveBeenCalled()

    await wrapper.setProps({ errorMarker: null })
    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(stub.model, 'sql-explorer', [])
  })

  it('sets no mark when the editor has no model', () => {
    const stub = stubEditor()
    stub.editor.getModel.mockReturnValue(null as never)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    vi.mocked(monaco.editor.setModelMarkers).mockClear()
    mount(SqlEditor, {
      props: { modelValue: '', errorMarker: { line: 1, column: 1, message: 'x' } },
    })
    expect(monaco.editor.setModelMarkers).not.toHaveBeenCalled()
  })

  it('moves the cursor to a place and shows it', () => {
    const stub = stubEditor()
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    const wrapper = mount(SqlEditor, { props: { modelValue: '' } })
    const vm = wrapper.vm as unknown as { reveal: (line: number, column: number) => void }
    vm.reveal(3, 4)
    expect(stub.editor.setPosition).toHaveBeenCalledWith({ lineNumber: 3, column: 4 })
    expect(stub.editor.revealLineInCenter).toHaveBeenCalledWith(3)
    expect(stub.editor.focus).toHaveBeenCalled()

    wrapper.unmount()
    // The editor is gone, so the call does nothing.
    vm.reveal(1, 1)
  })

  it('gives the text to run with the place where it begins', () => {
    type Run = { text: string; start?: { line: number; column: number } }
    const runOf = (wrapper: { vm: unknown }) =>
      (wrapper.vm as { currentRun: () => Run }).currentRun()

    const stub = stubEditor('SELECT 1;\nSELECT 2')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    stub.model.getOffsetAt.mockReturnValue(12)
    const wrapper = mount(SqlEditor, {
      props: { modelValue: 'SELECT 1;\nSELECT 2', dialect: Dialect.Postgres },
    })
    expect(runOf(wrapper)).toEqual({ text: 'SELECT 2', start: { line: 2, column: 1 } })

    // A cursor in the blank space before a statement finds it further on.
    stub.setValue('\n\nSELECT 1')
    stub.model.getOffsetAt.mockReturnValue(0)
    expect(runOf(wrapper).text).toBe('SELECT 1')
    expect(stub.model.getPositionAt).toHaveBeenLastCalledWith(2)

    // Two statements start with the same line, and the cursor stands in
    // the comment above the second. The start is the comment of the second.
    const twins = 'SELECT a\nFROM t;\n-- pick\nSELECT a\nFROM u'
    stub.setValue(twins)
    stub.model.getOffsetAt.mockReturnValue(twins.indexOf('pick'))
    expect(runOf(wrapper).start).toEqual({ line: 3, column: 1 })
    // The cursor in the blank line above a copy of the first statement.
    stub.setValue('SELECT a;\n\nSELECT a')
    stub.model.getOffsetAt.mockReturnValue(10)
    expect(runOf(wrapper)).toEqual({ text: 'SELECT a', start: { line: 3, column: 1 } })

    // An empty text has no place.
    stub.setValue('')
    expect(runOf(wrapper)).toEqual({ text: '' })

    stub.editor.getSelection.mockReturnValue({
      isEmpty: () => false,
      startLineNumber: 3,
      startColumn: 2,
    } as never)
    expect(runOf(wrapper)).toEqual({ text: 'SELECTED', start: { line: 3, column: 2 } })
  })

  it('counts the DELIMITER line in front of a MySQL routine', () => {
    type Run = { text: string; start?: { line: number; column: number } }
    const runOf = (wrapper: { vm: unknown }) =>
      (wrapper.vm as { currentRun: () => Run }).currentRun()
    const script = 'DELIMITER $$\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\nEND$$\nDELIMITER ;'
    const stub = stubEditor(script)
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    stub.model.getOffsetAt.mockReturnValue(script.indexOf('BEGIN'))
    const wrapper = mount(SqlEditor, { props: { modelValue: script, dialect: Dialect.MySql } })
    // Line 2 of the sent text is line 2 of the editor, so the start is the
    // line above the routine.
    expect(runOf(wrapper)).toEqual({
      text: 'DELIMITER $$\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\nEND$$',
      start: { line: 1, column: 1 },
    })

    // A routine that starts inside a line gets no place.
    const inline = 'DELIMITER $$\nSELECT 1$$ SELECT 2; SELECT 3$$'
    stub.setValue(inline)
    stub.model.getOffsetAt.mockReturnValue(inline.indexOf('SELECT 3'))
    expect(runOf(wrapper)).toEqual({ text: 'DELIMITER $$\nSELECT 2; SELECT 3$$' })
  })

  it('tells the parent when an edit removes the mark of a failure', async () => {
    const stub = stubEditor('SELECT x\nFROM t')
    vi.mocked(monaco.editor.create).mockReturnValue(asEditor(stub.editor))
    vi.mocked(monaco.editor.setModelMarkers).mockClear()
    const wrapper = mount(SqlEditor, {
      props: { modelValue: 'SELECT x\nFROM t', errorMarker: { line: 2, column: 30, message: 'x' } },
    })
    // A column past the end of the line moves to the end of that line.
    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(stub.model, 'sql-explorer', [
      expect.objectContaining({ startLineNumber: 2, startColumn: 12, endColumn: 12 }),
    ])
    stub.fireContentChange()
    expect(wrapper.emitted('marker-cleared')).toHaveLength(1)
    stub.fireContentChange()
    expect(wrapper.emitted('marker-cleared')).toHaveLength(1)

    // A line past the end of the text gets no mark.
    await wrapper.setProps({ errorMarker: { line: 3, column: 1, message: 'gone' } })
    expect(monaco.editor.setModelMarkers).toHaveBeenLastCalledWith(stub.model, 'sql-explorer', [])
    stub.fireContentChange()
    expect(wrapper.emitted('marker-cleared')).toHaveLength(1)
  })
})
