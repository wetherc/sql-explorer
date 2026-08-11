import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createVuetify } from 'vuetify'
import { VIcon } from 'vuetify/components'
import type { IconProps } from 'vuetify'
import { aliases, iconPaths, mdi, pathFor } from '../icons'

/** The text of every file of the source tree, without the tests. */
const sources = import.meta.glob('../../**/*.{ts,vue}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

describe('the icons of the application', () => {
  it('holds a path for every name that the source names', () => {
    const names = new Set<string>()
    for (const [path, text] of Object.entries(sources)) {
      // The record itself names the icon set of Vuetify, which is no icon,
      // and a test can name an icon that the application never draws.
      if (path.includes('__tests__') || path.endsWith('.spec.ts')) {
        continue
      }
      if (path.endsWith('/icons.ts')) {
        continue
      }
      for (const match of text.matchAll(/mdi-[a-z0-9-]+/g)) {
        names.add(match[0])
      }
    }

    // The record holds these names alone, so the comparison also finds a
    // name that no file uses any more.
    expect([...names].sort()).toEqual(Object.keys(iconPaths).sort())
    for (const path of Object.values(iconPaths)) {
      expect(path.startsWith('M')).toBe(true)
    }
  })

  it('lets a path and an alias of Vuetify through', () => {
    expect(pathFor('mdi-play')).toBe(iconPaths['mdi-play'])
    // The aliases of Vuetify already carry a path.
    expect(pathFor('M0 0h24v24H0z')).toBe('M0 0h24v24H0z')
    expect(pathFor('mdi-not-in-the-record')).toBe('mdi-not-in-the-record')
  })

  it('draws the name that a component gives as a path', () => {
    const vuetify = createVuetify({
      icons: { defaultSet: 'mdi', aliases, sets: { mdi } },
    })
    const wrapper = mount(VIcon, {
      props: { icon: 'mdi-play' },
      global: { plugins: [vuetify] },
    })
    expect(wrapper.find('path').attributes('d')).toBe(iconPaths['mdi-play'])
  })

  it('leaves an icon that is not a name alone', () => {
    const component = { render: () => null } as unknown as IconProps['icon']
    expect(pathFor(component)).toBe(component)
  })
})
