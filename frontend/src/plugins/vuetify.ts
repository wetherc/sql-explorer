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

/**
 * The light theme holds no pure white. A window of white panels beside a
 * white editor is bright enough to tire a reader who works in it for a day,
 * so each surface stands a step below white and the text on it stands a step
 * darker than the text of a common light theme. The colours of the marks,
 * such as the keyword and the error, are darker for the same reason: a light
 * surface needs a deeper ink than a dark surface does.
 */
export const sqlExplorerLight = {
  dark: false,
  colors: {
    background: '#e7ebf0',
    surface: '#f6f8fa',
    'surface-bright': '#fbfcfd',
    'surface-light': '#eceff4',
    'surface-variant': '#ccd3de',
    'on-surface-variant': '#333c49',
    primary: '#1558c4',
    'primary-darken-1': '#10469c',
    secondary: '#4a5462',
    accent: '#6f3fc4',
    error: '#b21e30',
    info: '#0a5fb4',
    success: '#146b3f',
    warning: '#7d5300',
    'editor-background': '#f6f8fa',
    'grid-header': '#e6eaf0',
    'grid-stripe': '#f1f4f8',
    'on-grid-header': '#333c49',
    'on-grid-stripe': '#333c49',
    'null-value': '#636c77',
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
