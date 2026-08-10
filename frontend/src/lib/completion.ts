import { monaco } from '@/plugins/monaco'
import {
  completionsFor,
  emptySchemaIndex,
  qualifierBefore,
  tableAliases,
  wordBefore,
  type SchemaIndex,
} from '@/lib/sql'
import { Dialect } from '@/types/api'

/** What one editor knows about the names it can offer. */
export interface CompletionSource {
  index: SchemaIndex
  dialect: Dialect
}

/**
 * The source of each open model, keyed by the address of that model.
 *
 * The provider below belongs to the SQL language and not to one editor, so
 * one registration serves every tab. Each editor puts its own source in this
 * map, and the provider reads the source of the model it is asked about.
 */
const sources = new Map<string, () => CompletionSource>()

let provider: monaco.IDisposable | null = null

/** Records the source of one model. */
export function setCompletionSource(uri: string, source: () => CompletionSource): void {
  sources.set(uri, source)
}

/** Forgets the source of a model that is gone. */
export function clearCompletionSource(uri: string): void {
  sources.delete(uri)
}

/** The Monaco kind that matches one kind of name. */
export function monacoKind(kind: string): monaco.languages.CompletionItemKind {
  const kinds = monaco.languages.CompletionItemKind
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

/**
 * Builds the answer for one request of the editor. The function takes the
 * model and the position alone, so a test can call it without an editor.
 */
export function suggestionsFor(
  model: monaco.editor.ITextModel,
  position: monaco.Position,
): monaco.languages.CompletionList {
  const source = sources.get(model.uri.toString())?.() ?? {
    index: emptySchemaIndex(),
    dialect: Dialect.MsSql,
  }
  const text = model.getValue()
  const offset = model.getOffsetAt(position)
  const word = model.getWordUntilPosition(position)
  const range = {
    startLineNumber: position.lineNumber,
    endLineNumber: position.lineNumber,
    startColumn: word.startColumn,
    endColumn: word.endColumn,
  }

  const items = completionsFor(wordBefore(text, offset), source.index, source.dialect, {
    qualifier: qualifierBefore(text, offset),
    aliases: tableAliases(text, source.dialect),
  })

  return {
    suggestions: items.map((item) => ({
      label: item.label,
      detail: item.detail,
      insertText: item.insertText,
      kind: monacoKind(item.kind),
      range,
    })),
  }
}

/**
 * Registers the provider of the SQL language, once for the application. A
 * second call does nothing, so a tab that opens adds no second list of the
 * same names.
 *
 * The full stop is a trigger, because the name after a full stop is the one
 * the user most often wants.
 */
export function installSqlCompletions(): void {
  if (provider) {
    return
  }
  provider = monaco.languages.registerCompletionItemProvider('sql', {
    triggerCharacters: ['.'],
    provideCompletionItems: suggestionsFor,
  })
}

/** Drops the provider. The tests use this to start from nothing. */
export function disposeSqlCompletions(): void {
  provider?.dispose()
  provider = null
  sources.clear()
}
