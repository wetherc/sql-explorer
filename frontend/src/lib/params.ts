/**
 * The values that the user gives for the named parameters of a statement.
 *
 * A value carries the form the user chose, so no text is read as a number by
 * chance. An identifier such as `007` therefore stays as it is.
 */
import { ParamKind, type ParamValue } from '@/types/api'

/** Builds the record for one name, with the form and the text it starts at. */
export function newParamValue(name: string, held?: ParamValue): ParamValue {
  return held ? { ...held } : { name, kind: ParamKind.Text, text: '' }
}

/** Turns one value of the dialog into the JSON that the backend binds. */
export function jsonOfParam(value: ParamValue): unknown {
  if (value.kind === ParamKind.Null) {
    return null
  }
  if (value.kind === ParamKind.Boolean) {
    return value.text.trim().toLowerCase() === 'true'
  }
  if (value.kind === ParamKind.Number) {
    // The dialog refuses a text that is not a number, so this call never
    // meets one. A text that still arrives keeps its own form, and the
    // server judges it.
    const number = Number(value.text.trim())
    return Number.isFinite(number) ? number : value.text
  }
  return value.text
}

/**
 * True when the text of a value does not fit the form the user chose. The
 * dialog blocks its confirm button while one row is wrong.
 */
export function paramProblem(value: ParamValue): string | null {
  if (value.kind !== ParamKind.Number) {
    return null
  }
  const text = value.text.trim()
  if (text === '') {
    return null
  }
  return Number.isFinite(Number(text)) ? null : 'Write a number.'
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
  if (held.kind === ParamKind.Null) {
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
  return value.kind !== ParamKind.Null && value.text.trim() === ''
}

/** Reads the parameter values of a tab out of the workspace file. */
export function parseParamValues(value: unknown): ParamValue[] {
  if (!Array.isArray(value)) {
    return []
  }
  const kinds: string[] = Object.values(ParamKind)
  return value
    .filter((item): item is Record<string, unknown> => typeof item === 'object' && item !== null)
    .filter((item) => typeof item.name === 'string' && typeof item.text === 'string')
    .map((item) => ({
      name: item.name as string,
      kind: (kinds.includes(item.kind as string) ? item.kind : ParamKind.Text) as ParamKind,
      text: item.text as string,
    }))
}
