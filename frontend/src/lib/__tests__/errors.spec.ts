import { describe, expect, it } from 'vitest'
import {
  errorAdvice,
  errorIcon,
  fullErrorText,
  isCancellation,
  isErrorPayload,
  toErrorPayload,
} from '@/lib/errors'
import { ErrorCategory } from '@/types/api'

describe('toErrorPayload', () => {
  it('keeps a payload the backend sent', () => {
    const payload = { category: ErrorCategory.Database, message: 'bad column', detail: 'line 1' }
    expect(toErrorPayload(payload)).toBe(payload)
  })

  it('wraps a fault of the bridge itself', () => {
    const result = toErrorPayload(new Error('the bridge is closed'))
    expect(result).toEqual({
      category: ErrorCategory.Internal,
      message: 'the bridge is closed',
      detail: null,
    })
  })

  it('wraps a plain text', () => {
    expect(toErrorPayload('boom')).toEqual({
      category: ErrorCategory.Internal,
      message: 'boom',
      detail: null,
    })
  })

  it('wraps a value it cannot read', () => {
    const result = toErrorPayload({ unexpected: 1 })
    expect(result.category).toBe(ErrorCategory.Internal)
    expect(result.detail).toBe('{"unexpected":1}')
  })

  it('wraps a value that cannot become JSON', () => {
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    expect(toErrorPayload(cyclic).detail).toBe('[object Object]')
  })

  it('wraps a value that becomes no JSON text', () => {
    expect(toErrorPayload(undefined).detail).toBeNull()
  })
})

describe('isErrorPayload', () => {
  it('accepts an object with a category and a message', () => {
    expect(isErrorPayload({ category: 'database', message: 'x' })).toBe(true)
  })

  it('refuses anything else', () => {
    expect(isErrorPayload(null)).toBe(false)
    expect(isErrorPayload('text')).toBe(false)
    expect(isErrorPayload({ category: 1, message: 'x' })).toBe(false)
    expect(isErrorPayload({ category: 'database' })).toBe(false)
  })
})

describe('fullErrorText', () => {
  it('joins the message and the detail', () => {
    expect(fullErrorText({ category: ErrorCategory.Database, message: 'a', detail: 'b' })).toBe(
      'a\nb',
    )
  })

  it('gives the message alone when there is no detail', () => {
    expect(fullErrorText({ category: ErrorCategory.Database, message: 'a', detail: null })).toBe(
      'a',
    )
  })
})

describe('isCancellation', () => {
  it('holds only for a stopped operation', () => {
    expect(isCancellation({ category: ErrorCategory.Cancelled, message: '', detail: null })).toBe(
      true,
    )
    expect(isCancellation({ category: ErrorCategory.Database, message: '', detail: null })).toBe(
      false,
    )
  })
})

describe('errorIcon', () => {
  it('gives an icon for every category', () => {
    const categories = Object.values(ErrorCategory)
    for (const category of categories) {
      expect(errorIcon(category)).toMatch(/^mdi-/)
    }
    expect(errorIcon(ErrorCategory.NotConnected)).toBe('mdi-lan-disconnect')
    expect(errorIcon(ErrorCategory.Connection)).toBe('mdi-lan-disconnect')
    expect(errorIcon(ErrorCategory.Timeout)).toBe('mdi-timer-alert-outline')
    expect(errorIcon(ErrorCategory.Cancelled)).toBe('mdi-cancel')
    expect(errorIcon(ErrorCategory.Configuration)).toBe('mdi-tune')
    expect(errorIcon(ErrorCategory.Secret)).toBe('mdi-key-alert-outline')
    expect(errorIcon(ErrorCategory.Unsupported)).toBe('mdi-block-helper')
    expect(errorIcon(ErrorCategory.Io)).toBe('mdi-file-alert-outline')
    expect(errorIcon(ErrorCategory.Storage)).toBe('mdi-file-alert-outline')
    expect(errorIcon(ErrorCategory.Database)).toBe('mdi-alert-circle-outline')
  })
})

describe('errorAdvice', () => {
  const advise = (category: ErrorCategory) => errorAdvice({ category, message: '', detail: null })

  it('gives advice for the categories a user can act on', () => {
    expect(advise(ErrorCategory.NotConnected)).toContain('Reconnect')
    expect(advise(ErrorCategory.Connection)).toContain('host')
    expect(advise(ErrorCategory.Timeout)).toContain('timeout')
    expect(advise(ErrorCategory.Configuration)).toContain('Fix')
    expect(advise(ErrorCategory.Secret)).toContain('keychain')
    expect(advise(ErrorCategory.Io)).toContain('permission')
  })

  it('gives no advice when the message is enough', () => {
    expect(advise(ErrorCategory.Database)).toBe('')
    // A value the user typed is wrong, and the message says which one, so
    // the connection details are not the cause.
    expect(advise(ErrorCategory.Invalid)).toBe('')
  })
})

describe('the authentication category', () => {
  it('gets an icon and advice of its own', () => {
    const payload = {
      category: ErrorCategory.Authentication,
      message: 'The Azure CLI was not found.',
      detail: null,
    }
    expect(errorIcon(payload.category)).toBe('mdi-key-alert-outline')
    expect(errorAdvice(payload)).toContain('authentication method')
  })
})
