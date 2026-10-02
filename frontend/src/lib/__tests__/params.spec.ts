import { describe, expect, it } from 'vitest'
import {
  alignParams,
  booleanOfText,
  jsonOfParam,
  needsAValue,
  newParamValue,
  paramChipLabel,
  paramProblem,
  paramsForRun,
  parseParamValues,
} from '@/lib/params'
import { ParamType } from '@/types/api'

describe('newParamValue', () => {
  it('starts a value as text and keeps one that is held', () => {
    expect(newParamValue('id')).toEqual({ name: 'id', valueType: ParamType.Text, text: '' })

    const held = { name: 'id', valueType: ParamType.Number, text: '7' }
    const copy = newParamValue('id', held)
    expect(copy).toEqual(held)
    expect(copy).not.toBe(held)
  })
})

describe('jsonOfParam', () => {
  it('gives the form the user chose', () => {
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Text, text: '007' })).toBe('007')
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: ' 12 ' })).toBe(12)
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Boolean, text: 'True' })).toBe(true)
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Boolean, text: 'no' })).toBe(false)
    // A text that names neither state goes to the server as it is.
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Boolean, text: 'maybe' })).toBe('maybe')
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Null, text: 'ignored' })).toBeNull()
  })

  it('keeps the text of a number it cannot read, so no value goes missing', () => {
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: 'two' })).toBe('two')
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: '1e400' })).toBe('1e400')
  })

  it('sends the digits of a number that a double changes', () => {
    // A double holds 90071992547409936 for these digits.
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: '90071992547409931' })).toBe(
      '90071992547409931',
    )
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: '1.50' })).toBe('1.50')
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: '-0.1' })).toBe(-0.1)
    // The server does not read the exponent form in each type, so the value
    // stays a number.
    expect(jsonOfParam({ name: 'a', valueType: ParamType.Number, text: '1e3' })).toBe(1000)
  })
})

describe('booleanOfText', () => {
  it('reads each word of the two states and refuses another word', () => {
    for (const word of ['true', 'T', ' yes ', 'y', 'ON', '1']) {
      expect(booleanOfText(word)).toBe(true)
    }
    for (const word of ['false', 'F', 'no', 'N', 'off', '0']) {
      expect(booleanOfText(word)).toBe(false)
    }
    expect(booleanOfText('maybe')).toBeNull()
    expect(booleanOfText('')).toBeNull()
  })
})

describe('paramProblem', () => {
  it('names a text that the number form refuses', () => {
    expect(paramProblem({ name: 'a', valueType: ParamType.Number, text: 'two' })).toBe(
      'Write a number.',
    )
  })

  it('names a text that the true or false form refuses', () => {
    expect(paramProblem({ name: 'a', valueType: ParamType.Boolean, text: 'maybe' })).toBe(
      'Write true or false.',
    )
    expect(paramProblem({ name: 'a', valueType: ParamType.Boolean, text: ' Yes ' })).toBeNull()
    expect(paramProblem({ name: 'a', valueType: ParamType.Boolean, text: '  ' })).toBeNull()
  })

  it('finds no fault in a text that fits its form', () => {
    expect(paramProblem({ name: 'a', valueType: ParamType.Number, text: ' 12 ' })).toBeNull()
    // An empty box waits for the user, and the run itself asks for the value.
    expect(paramProblem({ name: 'a', valueType: ParamType.Number, text: '  ' })).toBeNull()
    expect(paramProblem({ name: 'a', valueType: ParamType.Text, text: 'two' })).toBeNull()
    expect(paramProblem({ name: 'a', valueType: ParamType.Null, text: 'two' })).toBeNull()
  })
})

describe('paramChipLabel', () => {
  it('names a value that the tab holds', () => {
    const values = [
      { name: 'id', valueType: ParamType.Number, text: '7' },
      { name: 'gone', valueType: ParamType.Null, text: '' },
    ]
    expect(paramChipLabel('id', values)).toBe(':id = 7')
    expect(paramChipLabel('gone', values)).toBe(':gone = empty value')
  })

  it('says that a value is still missing', () => {
    expect(paramChipLabel('id', [])).toBe(':id = unset')
    expect(paramChipLabel('id', [{ name: 'id', valueType: ParamType.Text, text: '  ' }])).toBe(
      ':id = unset',
    )
  })
})

describe('paramsForRun', () => {
  it('builds the map that travels beside the statement', () => {
    expect(
      paramsForRun([
        { name: 'id', valueType: ParamType.Number, text: '3' },
        { name: 'name', valueType: ParamType.Text, text: 'a' },
      ]),
    ).toEqual({ id: 3, name: 'a' })
  })
})

describe('alignParams', () => {
  it('keeps a value that is held and drops a name that is gone', () => {
    const held = [
      { name: 'id', valueType: ParamType.Number, text: '7' },
      { name: 'old', valueType: ParamType.Text, text: 'x' },
    ]
    expect(alignParams(['id', 'fresh'], held)).toEqual([
      { name: 'id', valueType: ParamType.Number, text: '7' },
      { name: 'fresh', valueType: ParamType.Text, text: '' },
    ])
  })
})

describe('needsAValue', () => {
  it('waits for text but not for an empty value', () => {
    expect(needsAValue({ name: 'a', valueType: ParamType.Text, text: '  ' })).toBe(true)
    expect(needsAValue({ name: 'a', valueType: ParamType.Text, text: 'x' })).toBe(false)
    expect(needsAValue({ name: 'a', valueType: ParamType.Null, text: '' })).toBe(false)
  })
})

describe('parseParamValues', () => {
  it('keeps the records that are usable and drops the rest', () => {
    expect(
      parseParamValues([
        { name: 'id', valueType: 'number', text: '7' },
        // An unknown form falls back to text.
        { name: 'name', valueType: 'colour', text: 'a' },
        { name: 'no-text' },
        { valueType: 'text', text: 'nameless' },
        'nonsense',
        null,
      ]),
    ).toEqual([
      { name: 'id', valueType: ParamType.Number, text: '7' },
      { name: 'name', valueType: ParamType.Text, text: 'a' },
    ])
  })

  it('reads the type of a value that a workspace of an earlier release saved', () => {
    expect(
      parseParamValues([
        { name: 'id', kind: 'number', text: '7' },
        { name: 'bare', text: 'x' },
      ]),
    ).toEqual([
      { name: 'id', valueType: ParamType.Number, text: '7' },
      { name: 'bare', valueType: ParamType.Text, text: 'x' },
    ])
  })

  it('gives an empty list for anything that is not a list', () => {
    expect(parseParamValues(undefined)).toEqual([])
    expect(parseParamValues({ name: 'id' })).toEqual([])
  })
})
