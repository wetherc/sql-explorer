import { describe, expect, it } from 'vitest'
import backendSource from '../../../../backend/src/history.rs?raw'
import { HISTORY_LIMIT, HISTORY_TEXT_BUDGET } from '../history'

/**
 * The value of one `pub const NAME: usize = ...;` line of the backend file.
 * The value is a whole number or a product of whole numbers, such as
 * `4 * 1024 * 1024`.
 */
function backendConstant(name: string): number {
  const match = new RegExp(`pub const ${name}: usize = ([\\d_\\s*]+);`).exec(backendSource)
  if (!match?.[1]) {
    throw new Error(`backend/src/history.rs has no constant ${name}`)
  }
  return match[1]
    .split('*')
    .map((factor) => Number(factor.replace(/_/g, '').trim()))
    .reduce((product, factor) => product * factor, 1)
}

// The backend trims the history file and the frontend trims its list. A
// difference makes the list drop entries that the file keeps, or the reverse.
describe('the history limits', () => {
  it('match the limits of the backend', () => {
    expect(backendConstant('HISTORY_LIMIT')).toBe(HISTORY_LIMIT)
    expect(backendConstant('HISTORY_TEXT_BUDGET')).toBe(HISTORY_TEXT_BUDGET)
  })

  it('fails on a constant that the backend lacks', () => {
    expect(() => backendConstant('NO_SUCH_LIMIT')).toThrow('has no constant NO_SUCH_LIMIT')
  })
})
