import { createPinia, setActivePinia } from 'pinia'
import { mount } from '@vue/test-utils'
import { createVuetify } from 'vuetify'
import * as components from 'vuetify/components'
import * as directives from 'vuetify/directives'
import type { Component } from 'vue'

/** One Vuetify instance serves every test, because building it is slow. */
const vuetify = createVuetify({ components, directives })

/** The options a test may give. They are loose, because a test names only
 * the props it cares about. */
export interface MountOptions {
  props?: Record<string, unknown>
  global?: Record<string, unknown>
}

/**
 * Mounts a component with the plugins the application uses, and attaches
 * it to the document so that the parts Vuetify renders in a portal, such
 * as a menu or a dialog, can be found.
 */
export function mountWithPlugins(component: Component, options: MountOptions = {}) {
  setActivePinia(createPinia())
  return mount(component, {
    props: options.props,
    attachTo: document.body,
    global: {
      plugins: [vuetify],
      ...(options.global ?? {}),
    },
  })
}

/** Waits until Vuetify has drawn what a click opened. */
export async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
