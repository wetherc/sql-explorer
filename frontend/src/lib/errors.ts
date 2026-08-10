import { ErrorKind, type ErrorPayload } from '@/types/api'

/**
 * Turns whatever a failed command threw into the payload the interface
 * shows. The backend sends an object with a kind, a message and a detail.
 * Anything else, such as a fault inside the bridge itself, is wrapped so
 * that the caller always has the same three fields.
 */
export function toErrorPayload(error: unknown): ErrorPayload {
  if (isErrorPayload(error)) {
    return error
  }
  if (error instanceof Error) {
    return { kind: ErrorKind.Internal, message: error.message, detail: null }
  }
  if (typeof error === 'string') {
    return { kind: ErrorKind.Internal, message: error, detail: null }
  }
  return {
    kind: ErrorKind.Internal,
    message: 'The operation failed for a reason the application could not read.',
    detail: safeJson(error),
  }
}

/** True when the value has the three fields the backend sends. */
export function isErrorPayload(value: unknown): value is ErrorPayload {
  if (typeof value !== 'object' || value === null) {
    return false
  }
  const candidate = value as Record<string, unknown>
  return typeof candidate.kind === 'string' && typeof candidate.message === 'string'
}

/** Writes a value as JSON, and falls back on its text form. */
function safeJson(value: unknown): string | null {
  try {
    return JSON.stringify(value) ?? null
  } catch {
    return String(value)
  }
}

/** Joins the message and the detail into the text a dialog shows. */
export function fullErrorText(payload: ErrorPayload): string {
  return payload.detail ? `${payload.message}\n${payload.detail}` : payload.message
}

/** True when the user stopped the operation, so no alarm is needed. */
export function isCancellation(payload: ErrorPayload): boolean {
  return payload.kind === ErrorKind.Cancelled
}

/** Selects the icon that stands for the kind of a failure. */
export function errorIcon(kind: ErrorKind): string {
  switch (kind) {
    case ErrorKind.NotConnected:
    case ErrorKind.Connection:
      return 'mdi-lan-disconnect'
    case ErrorKind.Timeout:
      return 'mdi-timer-alert-outline'
    case ErrorKind.Cancelled:
      return 'mdi-cancel'
    case ErrorKind.Configuration:
      return 'mdi-tune'
    case ErrorKind.Secret:
      return 'mdi-key-alert-outline'
    case ErrorKind.Unsupported:
      return 'mdi-block-helper'
    case ErrorKind.Io:
    case ErrorKind.Storage:
      return 'mdi-file-alert-outline'
    default:
      return 'mdi-alert-circle-outline'
  }
}

/**
 * Gives advice on what to do next. An empty text means that the message
 * itself is enough.
 */
export function errorAdvice(payload: ErrorPayload): string {
  switch (payload.kind) {
    case ErrorKind.NotConnected:
      return 'Open the connection again from the connection list.'
    case ErrorKind.Connection:
      return 'Check the host, the port and the transport setting of the connection.'
    case ErrorKind.Timeout:
      return 'Raise the time limit in the connection options, or make the statement smaller.'
    case ErrorKind.Configuration:
      return 'Correct the connection details and try again.'
    case ErrorKind.Secret:
      return 'The keychain of the system refused the password. Type it again and save.'
    default:
      return ''
  }
}
