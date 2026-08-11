/**
 * The SQL language of the editor, which the package ships without a type
 * declaration of its own. The application reads the rules of this language,
 * adds one rule for a named parameter, and gives the result back to the
 * editor.
 */
declare module 'monaco-editor/esm/vs/basic-languages/sql/sql' {
  import type { languages } from 'monaco-editor'

  export const conf: languages.LanguageConfiguration
  export const language: languages.IMonarchLanguage & {
    tokenizer: { root: unknown[] } & Record<string, unknown>
  }
}
