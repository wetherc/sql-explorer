import { beforeEach, describe, expect, it, vi } from 'vitest'
import { monaco } from '@/plugins/monaco'
import type { editor as MonacoEditor } from 'monaco-editor'
import {
  clearCompletionSource,
  disposeSqlCompletions,
  installSqlCompletions,
  monacoKind,
  setCompletionSource,
  suggestionsFor,
} from '@/lib/completion'
import { emptySchemaIndex } from '@/lib/sql'
import { Dialect } from '@/types/api'

/** Builds a stand-in for a model of the editor. */
function stubModel(text: string, uri = 'model:one') {
  return {
    uri: { toString: () => uri },
    getValue: () => text,
    getOffsetAt: () => text.length,
    getWordUntilPosition: () => ({ startColumn: 1, endColumn: 4 }),
  } as unknown as MonacoEditor.ITextModel
}

const position = { lineNumber: 1, column: 4 } as monaco.Position

const index = {
  databases: ['Sales'],
  schemas: ['dbo'],
  tables: [{ name: 'orders', qualifier: 'Sales.dbo' }],
  columns: [
    { name: 'total', table: 'orders', qualifier: 'Sales.dbo', dataType: 'money' },
    { name: 'note', table: 'customers', qualifier: 'Sales.dbo', dataType: 'text' },
  ],
}

describe('completion provider', () => {
  beforeEach(() => {
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReset()
    vi.mocked(monaco.languages.registerCompletionItemProvider).mockReturnValue({
      dispose: vi.fn(),
    })
    disposeSqlCompletions()
  })

  it('registers once and asks for the full stop as a trigger', () => {
    installSqlCompletions()
    installSqlCompletions()
    const register = vi.mocked(monaco.languages.registerCompletionItemProvider)
    expect(register).toHaveBeenCalledTimes(1)
    expect(register.mock.calls[0]?.[1].triggerCharacters).toEqual(['.'])
  })

  it('answers from the source of the model it is asked about', () => {
    setCompletionSource('model:one', () => ({ index, dialect: Dialect.MsSql }))
    setCompletionSource('model:two', () => ({
      index: emptySchemaIndex(),
      dialect: Dialect.MsSql,
    }))

    const first = suggestionsFor(stubModel('SELECT tot', 'model:one'), position)
    expect(first.suggestions.map((item) => item.label)).toContain('total')

    const second = suggestionsFor(stubModel('SELECT tot', 'model:two'), position)
    expect(second.suggestions.map((item) => item.label)).not.toContain('total')
  })

  it('offers the keywords alone for a model it knows nothing about', () => {
    const answer = suggestionsFor(stubModel('sel', 'model:none'), position)
    expect(answer.suggestions.every((item) => item.detail === 'keyword')).toBe(true)
  })

  it('gives the columns of an alias after a full stop', () => {
    setCompletionSource('model:one', () => ({ index, dialect: Dialect.MsSql }))
    const answer = suggestionsFor(
      stubModel('SELECT * FROM Sales.dbo.orders AS o WHERE o.'),
      position,
    )
    expect(answer.suggestions.map((item) => item.label)).toEqual(['total'])
  })

  it('forgets a model that is gone', () => {
    setCompletionSource('model:one', () => ({ index, dialect: Dialect.MsSql }))
    clearCompletionSource('model:one')
    const answer = suggestionsFor(stubModel('SELECT tot'), position)
    expect(answer.suggestions.map((item) => item.label)).not.toContain('total')
  })

  it('marks each kind of name with its own icon', () => {
    const kinds = monaco.languages.CompletionItemKind
    expect(monacoKind('database')).toBe(kinds.Module)
    expect(monacoKind('schema')).toBe(kinds.Folder)
    expect(monacoKind('table')).toBe(kinds.Struct)
    expect(monacoKind('column')).toBe(kinds.Field)
    expect(monacoKind('keyword')).toBe(kinds.Keyword)
  })
})
