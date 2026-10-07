import { monaco } from '@/plugins/monaco'
import {
  completionsFor,
  emptySchemaIndex,
  qualifierBefore,
  statementAround,
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

/**
 * The largest number of names one answer carries. The list of the editor
 * shows a few rows and filters the names it holds as the user writes, so a
 * schema of many thousand columns needs no answer of that size.
 */
export const MAX_SUGGESTIONS = 300

/** Records the source of one model. */
export function setCompletionSource(uri: string, source: () => CompletionSource): void {
  sources.set(uri, source)
}

/** Forgets the source of a model that is gone. */
export function clearCompletionSource(uri: string): void {
  sources.delete(uri)
}

/** The Monaco icon that matches one type of name. */
export function completionIcon(nameType: string): monaco.languages.CompletionItemKind {
  const icons = monaco.languages.CompletionItemKind
  switch (nameType) {
    case 'database':
      return icons.Module
    case 'schema':
      return icons.Folder
    case 'table':
      return icons.Struct
    case 'column':
      return icons.Field
    default:
      return icons.Keyword
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
  const word = model.getWordUntilPosition(position)
  const range = {
    startLineNumber: position.lineNumber,
    endLineNumber: position.lineNumber,
    startColumn: word.startColumn,
    endColumn: word.endColumn,
  }

  // The names come from the statement that holds the cursor and not from the
  // whole file. The reader of the aliases splits that statement into
  // characters, so the cost of a keystroke follows one statement.
  const statement = statementAround(model.getValue(), model.getOffsetAt(position), source.dialect)
  const items = completionsFor(
    wordBefore(statement.text, statement.offset),
    source.index,
    source.dialect,
    {
      qualifier: qualifierBefore(statement.text, statement.offset),
      aliases: tableAliases(statement.text, source.dialect),
      limit: MAX_SUGGESTIONS,
    },
  )

  return {
    // A full list may leave out names that a longer prefix finds, so the
    // editor asks again on the next keystroke.
    incomplete: items.length >= MAX_SUGGESTIONS,
    suggestions: items.map((item) => ({
      label: item.label,
      detail: item.detail,
      insertText: item.insertText,
      kind: completionIcon(item.nameType),
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
