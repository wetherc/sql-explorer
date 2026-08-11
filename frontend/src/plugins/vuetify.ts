import 'vuetify/styles'
import { createVuetify } from 'vuetify'
import { aliases, mdi } from './icons'

/**
 * The two themes of the application. Every colour a component uses comes
 * from one of these tokens, so no component holds a colour of its own.
 */
export const sqlExplorerDark = {
  dark: true,
  colors: {
    background: '#14171c',
    surface: '#1b1f26',
    'surface-bright': '#242932',
    'surface-light': '#2c323d',
    'surface-variant': '#39404d',
    'on-surface-variant': '#c9d1de',
    primary: '#6aa9ff',
    'primary-darken-1': '#3d7fd6',
    secondary: '#8b9bb4',
    accent: '#c792ea',
    error: '#ff6b7a',
    info: '#5cc8ff',
    success: '#5ad19a',
    warning: '#f2c14e',
    'editor-background': '#171b21',
    'grid-header': '#242932',
    'grid-stripe': '#1f242c',
    // The theme builds no `on-` colour for a token of its own, so the two
    // surfaces of the grid name the text that stands on them.
    'on-grid-header': '#c9d1de',
    'on-grid-stripe': '#c9d1de',
    'null-value': '#7f8a9b',
  },
} as const

export const sqlExplorerLight = {
  dark: false,
  colors: {
    background: '#f4f6f9',
    surface: '#ffffff',
    'surface-bright': '#ffffff',
    'surface-light': '#eef1f6',
    'surface-variant': '#dde3ec',
    'on-surface-variant': '#3d4756',
    primary: '#1f6feb',
    'primary-darken-1': '#1a5ec4',
    secondary: '#5b6675',
    accent: '#8250df',
    error: '#c8253a',
    info: '#0969da',
    success: '#1a7f4b',
    warning: '#9a6700',
    'editor-background': '#ffffff',
    'grid-header': '#eef1f6',
    'grid-stripe': '#f7f9fc',
    'on-grid-header': '#3d4756',
    'on-grid-stripe': '#3d4756',
    'null-value': '#8892a0',
  },
} as const

export default createVuetify({
  theme: {
    defaultTheme: 'sqlExplorerDark',
    themes: {
      sqlExplorerDark,
      sqlExplorerLight,
    },
  },
  icons: {
    defaultSet: 'mdi',
    aliases,
    sets: { mdi },
  },
  defaults: {
    VBtn: { variant: 'text' },
    VTextField: { variant: 'outlined', density: 'compact', hideDetails: 'auto' },
    VSelect: { variant: 'outlined', density: 'compact', hideDetails: 'auto' },
    VTextarea: { variant: 'outlined', density: 'compact', hideDetails: 'auto' },
    VSwitch: { density: 'compact', hideDetails: 'auto', color: 'primary' },
    VCheckbox: { density: 'compact', hideDetails: 'auto', color: 'primary' },
  },
})
