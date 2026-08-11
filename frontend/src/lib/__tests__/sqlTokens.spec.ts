import { describe, expect, it } from 'vitest'
import { PARAMETER_RULES, rootWithParameters } from '@/lib/sqlTokens'

/** Reads the text that one rule of the list matches at the start of a text. */
function match(text: string): { token: string; text: string } | null {
  for (const [pattern, token] of PARAMETER_RULES) {
    const found = new RegExp(`^(?:${pattern.source})`).exec(text)
    if (found) {
      return { token, text: found[0] }
    }
  }
  return null
}

describe('PARAMETER_RULES', () => {
  it('marks a name that follows one colon', () => {
    expect(match(':id')).toEqual({ token: 'variable', text: ':id' })
    expect(match(':order_1 = 2')).toEqual({ token: 'variable', text: ':order_1' })
  })

  it('reads two colons as the cast of PostgreSQL', () => {
    expect(match('::text')).toEqual({ token: 'operator', text: '::' })
  })

  it('marks nothing when a colon carries no name', () => {
    expect(match(': 1')).toBeNull()
    expect(match('SELECT 1')).toBeNull()
  })
})

describe('rootWithParameters', () => {
  it('puts the rules after the rules of the quoted text', () => {
    const root = [{ include: '@comments' }, { include: '@strings' }, [/[;,.]/, 'delimiter']]

    const result = rootWithParameters(root)

    expect(result).toHaveLength(root.length + PARAMETER_RULES.length)
    expect(result[1]).toEqual({ include: '@strings' })
    expect(result.slice(2, 4)).toEqual(PARAMETER_RULES)
    expect(result[4]).toBe(root[2])
  })

  it('puts the rules at the front when there are no rules of quoted text', () => {
    const root = [{ include: '@comments' }]

    const result = rootWithParameters(root)

    expect(result.slice(0, 2)).toEqual(PARAMETER_RULES)
    expect(result[2]).toBe(root[0])
  })

  it('leaves the rules it was given as they were', () => {
    const root = [{ include: '@strings' }]
    rootWithParameters(root)
    expect(root).toHaveLength(1)
  })
})
