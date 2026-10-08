import { afterEach, describe, expect, it, vi } from 'vitest'
import { FALLBACK_CHAR_WIDTH, canvasContext, createTextMeter, fontOf } from '@/lib/textWidth'

function fakeContext() {
  const context = {
    font: '',
    measureText: vi.fn((text: string) => ({ width: text.length * 5 }) as TextMetrics),
  }
  return context
}

describe('createTextMeter', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('measures a text in its font and keeps the width', () => {
    const context = fakeContext()
    const meter = createTextMeter(context)
    expect(meter('abcd', '12px sans')).toBe(20)
    expect(context.font).toBe('12px sans')
    expect(meter('abcd', '12px sans')).toBe(20)
    expect(context.measureText).toHaveBeenCalledTimes(1)
  })

  it('estimates the width without a canvas or without a font', () => {
    expect(createTextMeter(null)('abc', '12px sans')).toBe(3 * FALLBACK_CHAR_WIDTH)
    const context = fakeContext()
    expect(createTextMeter(context)('abc', '')).toBe(3 * FALLBACK_CHAR_WIDTH)
    expect(context.measureText).not.toHaveBeenCalled()
  })

  it('forgets the width it used least recently when it keeps too many', () => {
    const context = fakeContext()
    const meter = createTextMeter(context)
    for (let index = 0; index < 50_000; index += 1) {
      meter(String(index), 'f')
    }
    // A use of the first width makes the second one the oldest.
    meter('0', 'f')
    meter('new', 'f')
    expect(context.measureText).toHaveBeenCalledTimes(50_001)
    meter('0', 'f')
    expect(context.measureText).toHaveBeenCalledTimes(50_001)
    meter('1', 'f')
    expect(context.measureText).toHaveBeenCalledTimes(50_002)
  })
})

describe('canvasContext', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('gives the context of the host', () => {
    expect(canvasContext()).toBeNull()
  })

  it('gives nothing when the host refuses a canvas', () => {
    vi.spyOn(document, 'createElement').mockImplementation(() => {
      throw new Error('no canvas')
    })
    expect(canvasContext()).toBeNull()
  })
})

describe('fontOf', () => {
  it('builds the font of an element from its style', () => {
    const element = document.createElement('span')
    element.style.font = 'italic bold 13px Arial'
    document.body.append(element)
    expect(fontOf(element)).toBe('italic bold 13px Arial')
    element.remove()
  })
})
