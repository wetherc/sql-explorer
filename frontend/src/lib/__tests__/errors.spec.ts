import { describe, expect, it } from 'vitest'
import {
  errorAdvice,
  errorIcon,
  fullErrorText,
  isCancellation,
  isErrorPayload,
  toErrorPayload,
} from '@/lib/errors'
import { ErrorKind } from '@/types/api'

describe('toErrorPayload', () => {
  it('keeps a payload the backend sent', () => {
    const payload = { kind: ErrorKind.Database, message: 'bad column', detail: 'line 1' }
    expect(toErrorPayload(payload)).toBe(payload)
  })

  it('wraps a fault of the bridge itself', () => {
    const result = toErrorPayload(new Error('the bridge is closed'))
    expect(result).toEqual({
      kind: ErrorKind.Internal,
      message: 'the bridge is closed',
      detail: null,
    })
  })

  it('wraps a plain text', () => {
    expect(toErrorPayload('boom')).toEqual({
      kind: ErrorKind.Internal,
      message: 'boom',
      detail: null,
    })
  })

  it('wraps a value it cannot read', () => {
    const result = toErrorPayload({ unexpected: 1 })
    expect(result.kind).toBe(ErrorKind.Internal)
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
  it('accepts an object with a kind and a message', () => {
    expect(isErrorPayload({ kind: 'database', message: 'x' })).toBe(true)
  })

  it('refuses anything else', () => {
    expect(isErrorPayload(null)).toBe(false)
    expect(isErrorPayload('text')).toBe(false)
    expect(isErrorPayload({ kind: 1, message: 'x' })).toBe(false)
    expect(isErrorPayload({ kind: 'database' })).toBe(false)
  })
})

describe('fullErrorText', () => {
  it('joins the message and the detail', () => {
    expect(fullErrorText({ kind: ErrorKind.Database, message: 'a', detail: 'b' })).toBe('a\nb')
  })

  it('gives the message alone when there is no detail', () => {
    expect(fullErrorText({ kind: ErrorKind.Database, message: 'a', detail: null })).toBe('a')
  })
})

describe('isCancellation', () => {
  it('holds only for a stopped operation', () => {
    expect(isCancellation({ kind: ErrorKind.Cancelled, message: '', detail: null })).toBe(true)
    expect(isCancellation({ kind: ErrorKind.Database, message: '', detail: null })).toBe(false)
  })
})

describe('errorIcon', () => {
  it('gives an icon for every kind', () => {
    const kinds = Object.values(ErrorKind)
    for (const kind of kinds) {
      expect(errorIcon(kind)).toMatch(/^mdi-/)
    }
    expect(errorIcon(ErrorKind.NotConnected)).toBe('mdi-lan-disconnect')
    expect(errorIcon(ErrorKind.Connection)).toBe('mdi-lan-disconnect')
    expect(errorIcon(ErrorKind.Timeout)).toBe('mdi-timer-alert-outline')
    expect(errorIcon(ErrorKind.Cancelled)).toBe('mdi-cancel')
    expect(errorIcon(ErrorKind.Configuration)).toBe('mdi-tune')
    expect(errorIcon(ErrorKind.Secret)).toBe('mdi-key-alert-outline')
    expect(errorIcon(ErrorKind.Unsupported)).toBe('mdi-block-helper')
    expect(errorIcon(ErrorKind.Io)).toBe('mdi-file-alert-outline')
    expect(errorIcon(ErrorKind.Storage)).toBe('mdi-file-alert-outline')
    expect(errorIcon(ErrorKind.Database)).toBe('mdi-alert-circle-outline')
  })
})

describe('errorAdvice', () => {
  const advise = (kind: ErrorKind) => errorAdvice({ kind, message: '', detail: null })

  it('gives advice for the kinds a user can act on', () => {
    expect(advise(ErrorKind.NotConnected)).toContain('Open the connection')
    expect(advise(ErrorKind.Connection)).toContain('host')
    expect(advise(ErrorKind.Timeout)).toContain('time limit')
    expect(advise(ErrorKind.Configuration)).toContain('Correct')
    expect(advise(ErrorKind.Secret)).toContain('keychain')
  })

  it('gives no advice when the message is enough', () => {
    expect(advise(ErrorKind.Database)).toBe('')
  })
})
