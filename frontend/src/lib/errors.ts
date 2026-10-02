import { ErrorCategory, type ErrorPayload } from '@/types/api'

/**
 * Turns whatever a failed command threw into the payload the interface
 * shows. The backend sends an object with a category, a message and a detail.
 * Anything else, such as a fault inside the bridge itself, is wrapped so
 * that the caller always has the same three fields.
 */
export function toErrorPayload(error: unknown): ErrorPayload {
  if (isErrorPayload(error)) {
    return error
  }
  if (error instanceof Error) {
    return { category: ErrorCategory.Internal, message: error.message, detail: null }
  }
  if (typeof error === 'string') {
    return { category: ErrorCategory.Internal, message: error, detail: null }
  }
  return {
    category: ErrorCategory.Internal,
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
  return typeof candidate.category === 'string' && typeof candidate.message === 'string'
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
  return payload.category === ErrorCategory.Cancelled
}

/** Selects the icon that stands for the category of a failure. */
export function errorIcon(category: ErrorCategory): string {
  switch (category) {
    case ErrorCategory.NotConnected:
    case ErrorCategory.Connection:
      return 'mdi-lan-disconnect'
    case ErrorCategory.Timeout:
      return 'mdi-timer-alert-outline'
    case ErrorCategory.Cancelled:
      return 'mdi-cancel'
    case ErrorCategory.Configuration:
      return 'mdi-tune'
    case ErrorCategory.Authentication:
    case ErrorCategory.Secret:
      return 'mdi-key-alert-outline'
    case ErrorCategory.Unsupported:
      return 'mdi-block-helper'
    case ErrorCategory.Io:
    case ErrorCategory.Storage:
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
  switch (payload.category) {
    case ErrorCategory.NotConnected:
      return 'Open the connection again from the connection list.'
    case ErrorCategory.Connection:
      return 'Check the host, the port and the transport setting of the connection.'
    case ErrorCategory.Timeout:
      return 'Raise the time limit in the connection options, or make the statement smaller.'
    case ErrorCategory.Configuration:
      return 'Correct the connection details and try again.'
    case ErrorCategory.Authentication:
      return 'Check the authentication method of the connection and the credentials it needs.'
    case ErrorCategory.Secret:
      return 'The keychain of the system refused the password. Type it again and save.'
    default:
      return ''
  }
}
