/// <reference types="vite/client" />
/// <reference types="vite-plugin-vuetify/client" />

// `splitpanes` ships no type declarations of its own.
declare module 'splitpanes' {
  import type { DefineComponent } from 'vue'
  export const Splitpanes: DefineComponent<Record<string, unknown>>
  export const Pane: DefineComponent<Record<string, unknown>>
}

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}
