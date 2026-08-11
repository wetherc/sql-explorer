import { describe, expect, it } from 'vitest'
import {
  alignParams,
  jsonOfParam,
  needsAValue,
  newParamValue,
  paramChipLabel,
  paramProblem,
  paramsForRun,
  parseParamValues,
} from '@/lib/params'
import { ParamKind } from '@/types/api'

describe('newParamValue', () => {
  it('starts a value as text and keeps one that is held', () => {
    expect(newParamValue('id')).toEqual({ name: 'id', kind: ParamKind.Text, text: '' })

    const held = { name: 'id', kind: ParamKind.Number, text: '7' }
    const copy = newParamValue('id', held)
    expect(copy).toEqual(held)
    expect(copy).not.toBe(held)
  })
})

describe('jsonOfParam', () => {
  it('gives the form the user chose', () => {
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Text, text: '007' })).toBe('007')
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Number, text: ' 12 ' })).toBe(12)
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Boolean, text: 'True' })).toBe(true)
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Boolean, text: 'no' })).toBe(false)
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Null, text: 'ignored' })).toBeNull()
  })

  it('keeps the text of a number it cannot read, so no value goes missing', () => {
    expect(jsonOfParam({ name: 'a', kind: ParamKind.Number, text: 'two' })).toBe('two')
  })
})

describe('paramProblem', () => {
  it('names a text that the number form refuses', () => {
    expect(paramProblem({ name: 'a', kind: ParamKind.Number, text: 'two' })).toBe('Write a number.')
  })

  it('finds no fault in a text that fits its form', () => {
    expect(paramProblem({ name: 'a', kind: ParamKind.Number, text: ' 12 ' })).toBeNull()
    // An empty box waits for the user, and the run itself asks for the value.
    expect(paramProblem({ name: 'a', kind: ParamKind.Number, text: '  ' })).toBeNull()
    expect(paramProblem({ name: 'a', kind: ParamKind.Text, text: 'two' })).toBeNull()
    expect(paramProblem({ name: 'a', kind: ParamKind.Null, text: 'two' })).toBeNull()
  })
})

describe('paramChipLabel', () => {
  it('names a value that the tab holds', () => {
    const values = [
      { name: 'id', kind: ParamKind.Number, text: '7' },
      { name: 'gone', kind: ParamKind.Null, text: '' },
    ]
    expect(paramChipLabel('id', values)).toBe(':id = 7')
    expect(paramChipLabel('gone', values)).toBe(':gone = empty value')
  })

  it('says that a value is still missing', () => {
    expect(paramChipLabel('id', [])).toBe(':id = unset')
    expect(paramChipLabel('id', [{ name: 'id', kind: ParamKind.Text, text: '  ' }])).toBe(
      ':id = unset',
    )
  })
})

describe('paramsForRun', () => {
  it('builds the map that travels beside the statement', () => {
    expect(
      paramsForRun([
        { name: 'id', kind: ParamKind.Number, text: '3' },
        { name: 'name', kind: ParamKind.Text, text: 'a' },
      ]),
    ).toEqual({ id: 3, name: 'a' })
  })
})

describe('alignParams', () => {
  it('keeps a value that is held and drops a name that is gone', () => {
    const held = [
      { name: 'id', kind: ParamKind.Number, text: '7' },
      { name: 'old', kind: ParamKind.Text, text: 'x' },
    ]
    expect(alignParams(['id', 'fresh'], held)).toEqual([
      { name: 'id', kind: ParamKind.Number, text: '7' },
      { name: 'fresh', kind: ParamKind.Text, text: '' },
    ])
  })
})

describe('needsAValue', () => {
  it('waits for text but not for an empty value', () => {
    expect(needsAValue({ name: 'a', kind: ParamKind.Text, text: '  ' })).toBe(true)
    expect(needsAValue({ name: 'a', kind: ParamKind.Text, text: 'x' })).toBe(false)
    expect(needsAValue({ name: 'a', kind: ParamKind.Null, text: '' })).toBe(false)
  })
})

describe('parseParamValues', () => {
  it('keeps the records that are usable and drops the rest', () => {
    expect(
      parseParamValues([
        { name: 'id', kind: 'number', text: '7' },
        // An unknown form falls back to text.
        { name: 'name', kind: 'colour', text: 'a' },
        { name: 'no-text' },
        { kind: 'text', text: 'nameless' },
        'nonsense',
        null,
      ]),
    ).toEqual([
      { name: 'id', kind: ParamKind.Number, text: '7' },
      { name: 'name', kind: ParamKind.Text, text: 'a' },
    ])
  })

  it('gives an empty list for anything that is not a list', () => {
    expect(parseParamValues(undefined)).toEqual([])
    expect(parseParamValues({ name: 'id' })).toEqual([])
  })
})
