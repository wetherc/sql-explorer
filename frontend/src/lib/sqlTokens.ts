/**
 * The rules that mark a named parameter in the editor.
 *
 * They follow the scanner of the backend: two colons together are the cast of
 * PostgreSQL and carry no name, and a name holds letters, numbers and the low
 * line. The backend stays the judge of what runs, so a drift between the two
 * shows a wrong colour and never a wrong statement.
 */
export const PARAMETER_RULES: [RegExp, string][] = [
  [/::/, 'operator'],
  [/:\w+/, 'variable'],
]

/** The mark of a rule that brings in another group of rules. */
interface IncludeRule {
  include?: string
}

/**
 * Puts the rules for a parameter into the rules of the language, after the
 * rules for the comments, the white space and the quoted text. A name inside
 * a comment or inside quotes therefore keeps the colour of the text that
 * holds it.
 *
 * The rules go at the front when the group of the quoted text is absent, so
 * that a language which changes still marks a parameter.
 */
export function rootWithParameters(root: unknown[]): unknown[] {
  const strings = root.findIndex(
    (rule) =>
      typeof rule === 'object' && rule !== null && (rule as IncludeRule).include === '@strings',
  )
  const at = strings + 1
  return [...root.slice(0, at), ...PARAMETER_RULES, ...root.slice(at)]
}
