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
    message: "The operation failed, and the app couldn't read the error.",
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
    case ErrorCategory.KeptGone:
      return 'mdi-history'
    default:
      return 'mdi-alert-circle-outline'
  }
}

/**
 * Gives advice on what to do next. An empty text means that the message
 * itself is enough.
 */
export function errorAdvice(payload: ErrorPayload): string {
  // The message names the VPN, and the host, port and transport of the
  // connection are not the cause.
  if (payload.reason === 'kerberosUnreachable') {
    return ''
  }
  switch (payload.category) {
    case ErrorCategory.NotConnected:
      return 'Reconnect from the Connections list.'
    case ErrorCategory.Connection:
      return "Check the connection's host, port, and transport settings."
    case ErrorCategory.Timeout:
      return 'Increase the timeout in the connection options, or try a smaller statement.'
    case ErrorCategory.Configuration:
      return 'Fix the connection details and try again.'
    case ErrorCategory.Authentication:
      return "Check the connection's authentication method and credentials."
    case ErrorCategory.Secret:
      return 'The system keychain rejected the password. Enter it again and save.'
    case ErrorCategory.Io:
      return 'Check that the file exists and that you have permission to use it.'
    default:
      return ''
  }
}
