/**
 * The values that the user gives for the named parameters of a statement.
 *
 * A value carries the form the user chose, so no text is read as a number by
 * chance. An identifier such as `007` therefore stays as it is.
 */
import { ParamType, type ParamValue } from '@/types/api'

/**
 * A number that holds digits alone, with a sign and a decimal point. The
 * digits of such a text go to the server as they are if a double changes
 * them. A text with an exponent stays a number, because the server does not
 * read that form in each type.
 */
const PLAIN_NUMBER = /^[+-]?(\d+(\.\d*)?|\.\d+)$/

/** The texts that a Boolean value accepts for true. */
const TRUE_WORDS = ['true', 't', 'yes', 'y', 'on', '1']

/** The texts that a Boolean value accepts for false. */
const FALSE_WORDS = ['false', 'f', 'no', 'n', 'off', '0']

/**
 * Reads the text of a Boolean value. A text that names neither state gives
 * `null`, so the dialog can hold the row as wrong and the value does not
 * become false by chance.
 */
export function booleanOfText(text: string): boolean | null {
  const word = text.trim().toLowerCase()
  if (TRUE_WORDS.includes(word)) {
    return true
  }
  if (FALSE_WORDS.includes(word)) {
    return false
  }
  return null
}

/** Builds the record for one name, with the form and the text it starts at. */
export function newParamValue(name: string, held?: ParamValue): ParamValue {
  return held ? { ...held } : { name, valueType: ParamType.Text, text: '' }
}

/** Turns one value of the dialog into the JSON that the backend binds. */
export function jsonOfParam(value: ParamValue): unknown {
  if (value.valueType === ParamType.Null) {
    return null
  }
  if (value.valueType === ParamType.Boolean) {
    // A text that names neither state goes to the server as it is, and the
    // server judges it. The dialog blocks such a text before the run.
    const flag = booleanOfText(value.text)
    return flag === null ? value.text : flag
  }
  if (value.valueType === ParamType.Number) {
    // A double does not hold every number that the user can write. The value
    // goes as a number only if its digits come back unchanged. If they do
    // not, the digits go as text and the server reads them. A text that the
    // dialog refuses also goes as text, and the server judges it.
    const text = value.text.trim()
    const number = Number(text)
    if (!Number.isFinite(number)) {
      return text
    }
    return PLAIN_NUMBER.test(text) && String(number) !== text ? text : number
  }
  return value.text
}

/**
 * True when the text of a value does not fit the form the user chose. The
 * dialog blocks its confirm button while one row is wrong.
 */
export function paramProblem(value: ParamValue): string | null {
  const text = value.text.trim()
  if (text === '') {
    return null
  }
  if (value.valueType === ParamType.Number) {
    return Number.isFinite(Number(text)) ? null : 'Enter a number.'
  }
  if (value.valueType === ParamType.Boolean) {
    return booleanOfText(text) === null ? 'Enter true or false.' : null
  }
  return null
}

/**
 * The words that name one parameter and its value in the bar above the
 * editor. A name that waits for the user reads as unset, and an empty value
 * reads as the words that the dialog gives it.
 */
export function paramChipLabel(name: string, values: ParamValue[]): string {
  const held = values.find((value) => value.name === name)
  if (!held || needsAValue(held)) {
    return `:${name} = unset`
  }
  if (held.valueType === ParamType.Null) {
    return `:${name} = empty value`
  }
  return `:${name} = ${held.text}`
}

/** Builds the map of values that a run sends beside the statement. */
export function paramsForRun(values: ParamValue[]): Record<string, unknown> {
  const map: Record<string, unknown> = {}
  for (const value of values) {
    map[value.name] = jsonOfParam(value)
  }
  return map
}

/**
 * Lines up the values of a tab against the names that the statement holds.
 *
 * A name that the tab already holds keeps its value, so a second run needs no
 * dialog. A name that the statement no longer holds goes.
 */
export function alignParams(names: string[], held: ParamValue[]): ParamValue[] {
  return names.map((name) =>
    newParamValue(
      name,
      held.find((value) => value.name === name),
    ),
  )
}

/** True when a value is still waiting for the user. */
export function needsAValue(value: ParamValue): boolean {
  return value.valueType !== ParamType.Null && value.text.trim() === ''
}

/** Reads the parameter values of a tab out of the workspace file. */
export function parseParamValues(value: unknown): ParamValue[] {
  if (!Array.isArray(value)) {
    return []
  }
  const types: string[] = Object.values(ParamType)
  return value
    .filter((item): item is Record<string, unknown> => typeof item === 'object' && item !== null)
    .filter((item) => typeof item.name === 'string' && typeof item.text === 'string')
    .map((item) => {
      // The workspace file of an earlier release names the type in the field
      // `kind`. A value without `valueType` reads that field, so a saved tab
      // keeps the types of its values.
      const given = 'valueType' in item ? item.valueType : item.kind
      return {
        name: item.name as string,
        valueType: (types.includes(given as string) ? given : ParamType.Text) as ParamType,
        text: item.text as string,
      }
    })
}
