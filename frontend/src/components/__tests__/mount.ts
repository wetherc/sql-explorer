import { createPinia, setActivePinia } from 'pinia'
import { mount, type MountingOptions } from '@vue/test-utils'
import { createVuetify } from 'vuetify'
import * as components from 'vuetify/components'
import * as directives from 'vuetify/directives'
import type { Component } from 'vue'

/** One Vuetify instance serves every test, because building it is slow. */
const vuetify = createVuetify({ components, directives })

/**
 * Mounts a component with the plugins the application uses, and attaches
 * it to the document so that the parts Vuetify renders in a portal, such
 * as a menu or a dialog, can be found.
 */
export function mountWithPlugins<C extends Component>(
  component: C,
  options: MountingOptions<Record<string, unknown>> = {},
) {
  setActivePinia(createPinia())
  return mount(component, {
    ...options,
    attachTo: document.body,
    global: {
      plugins: [vuetify],
      stubs: { transition: false, 'transition-group': false },
      ...(options.global ?? {}),
    },
  } as MountingOptions<Record<string, unknown>>)
}

/** Waits until Vuetify has drawn what a click opened. */
export async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
