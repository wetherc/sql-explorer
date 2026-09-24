/** The width of one character when no canvas can measure a text. */
export const FALLBACK_CHAR_WIDTH = 7

/** The number of widths that a meter keeps before it starts again. */
const KEPT_WIDTHS = 50_000

/** Gives the width of a text in pixels, in one font. */
export type TextMeter = (text: string, font: string) => number

/** The part of a canvas context that a meter uses. */
export type MeasureContext = Pick<CanvasRenderingContext2D, 'font' | 'measureText'>

/**
 * Builds a function that gives the width of a text in one font. A canvas
 * measures a text without a layout of the page, so a list of thousands of
 * names costs little. The meter keeps each width, because a list measures
 * the same names again on each change. Without a canvas or a font, the
 * width is an estimate from the number of characters.
 */
export function createTextMeter(context: MeasureContext | null): TextMeter {
  const widths = new Map<string, number>()
  return (text, font) => {
    const key = `${font}\n${text}`
    const kept = widths.get(key)
    if (kept !== undefined) {
      return kept
    }
    let width = text.length * FALLBACK_CHAR_WIDTH
    if (context && font) {
      context.font = font
      width = context.measureText(text).width
    }
    if (widths.size >= KEPT_WIDTHS) {
      widths.clear()
    }
    widths.set(key, width)
    return width
  }
}

/** The 2D context of a new canvas, or `null` where the host gives none. */
export function canvasContext(): MeasureContext | null {
  try {
    return document.createElement('canvas').getContext('2d')
  } catch {
    return null
  }
}

/** The font of an element, in the form that a canvas context accepts. */
export function fontOf(element: Element): string {
  const style = getComputedStyle(element)
  return `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
}
