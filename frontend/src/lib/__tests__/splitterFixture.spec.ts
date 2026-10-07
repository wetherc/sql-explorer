import { describe, expect, it } from 'vitest'
import fixture from '../../../../tests/fixtures/splitter.json'
import type { Dialect } from '@/types/api'
import { statementAt, statementBounds } from '../sql'

/**
 * The statements of a script as the frontend finds them. The bounds also
 * cover the comments and the blank text between statements, and `statementAt`
 * gives an empty text for a span that contains no code, so such a span drops out.
 */
function statementsOf(script: string, dialect: Dialect): string[] {
  return statementBounds(script, dialect)
    .map(([start, end]) => script.slice(start, end))
    .filter((text) => statementAt(text, 0, dialect) !== '')
    .map((text) => text.trim())
}

// The backend reads the same file in a test of backend/src/sql.rs, so both
// splitters must find the same statements in each script.
describe('the shared splitter fixture', () => {
  for (const testCase of fixture.cases) {
    for (const dialect of testCase.dialects) {
      it(`${testCase.name} (${dialect})`, () => {
        expect(statementsOf(testCase.script, dialect as Dialect)).toEqual(testCase.statements)
      })
    }
  }
})
