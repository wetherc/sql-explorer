import { describe, expect, it } from 'vitest'
import { sqlExplorerDark, sqlExplorerLight } from '@/plugins/vuetify'

/** Turns a colour such as `#1b1f26` into its three channels from 0 to 1. */
function channels(hex: string): number[] {
  return [1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16) / 255)
}

function luminance(rgb: number[]): number {
  const [r, g, b] = rgb.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
}

/** The WCAG contrast of two colours given as channels. */
function contrast(a: number[], b: number[]): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (high! + 0.05) / (low! + 0.05)
}

/** A colour drawn at an opacity over another one, as the selected row is. */
function over(top: string, bottom: string, alpha: number): number[] {
  const [t, b] = [channels(top), channels(bottom)]
  return t.map((value, index) => value * alpha + b[index]! * (1 - alpha))
}

describe('the themes', () => {
  it.each([
    ['dark', sqlExplorerDark.colors],
    ['light', sqlExplorerLight.colors],
  ])('keep a NULL readable on every row of the %s grid', (_name, colors) => {
    const ink = channels(colors['null-value'])
    expect(contrast(ink, channels(colors.surface))).toBeGreaterThanOrEqual(4.5)
    expect(contrast(ink, channels(colors['grid-stripe']))).toBeGreaterThanOrEqual(4.5)
    const selected = over(colors.primary, colors['grid-stripe'], 0.16)
    expect(contrast(ink, selected)).toBeGreaterThanOrEqual(4.5)
  })

  it('puts dark text on the light primary and error colours of the dark theme', () => {
    const colors = sqlExplorerDark.colors
    expect(contrast(channels(colors['on-primary']), channels(colors.primary))).toBeGreaterThan(4.5)
    expect(contrast(channels(colors['on-error']), channels(colors.error))).toBeGreaterThan(4.5)
  })
})
