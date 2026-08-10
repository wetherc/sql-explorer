import * as monaco from 'monaco-editor'
import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import { sqlExplorerDark, sqlExplorerLight } from './vuetify'

/**
 * Points the editor at its worker. Only the base worker is needed, because
 * the editor handles SQL without a language server.
 */
export function configureMonacoEnvironment(): void {
  ;(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
    getWorker: () => new EditorWorker(),
  }
}

/** Registers the two themes that match the themes of the application. */
export function registerMonacoThemes(): void {
  monaco.editor.defineTheme('sql-explorer-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'keyword', foreground: '6aa9ff' },
      { token: 'string.sql', foreground: '5ad19a' },
      { token: 'comment', foreground: '7f8a9b', fontStyle: 'italic' },
      { token: 'number', foreground: 'f2c14e' },
      { token: 'operator.sql', foreground: 'c792ea' },
    ],
    colors: {
      'editor.background': sqlExplorerDark.colors['editor-background'],
      'editorLineNumber.foreground': sqlExplorerDark.colors['null-value'],
      'editor.lineHighlightBackground': sqlExplorerDark.colors['surface-bright'],
    },
  })

  monaco.editor.defineTheme('sql-explorer-light', {
    base: 'vs',
    inherit: true,
    rules: [
      { token: 'keyword', foreground: '1f6feb' },
      { token: 'string.sql', foreground: '1a7f4b' },
      { token: 'comment', foreground: '8892a0', fontStyle: 'italic' },
      { token: 'number', foreground: '9a6700' },
      { token: 'operator.sql', foreground: '8250df' },
    ],
    colors: {
      'editor.background': sqlExplorerLight.colors['editor-background'],
      'editorLineNumber.foreground': sqlExplorerLight.colors['null-value'],
      'editor.lineHighlightBackground': sqlExplorerLight.colors['surface-light'],
    },
  })
}

export { monaco }
