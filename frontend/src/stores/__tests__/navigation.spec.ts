// frontend/src/stores/__tests__/navigation.spec.ts
import { describe, it, expect, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useNavigationStore, type View } from '../navigation'

describe('navigation store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('initial state is correct', () => {
    const store = useNavigationStore()
    expect(store.activeView).toBe('explorer')
  })

  it('setActiveView updates the active view', () => {
    const store = useNavigationStore()
    const newView: View = 'connections'
    store.setActiveView(newView)
    expect(store.activeView).toBe(newView)
  })
})
