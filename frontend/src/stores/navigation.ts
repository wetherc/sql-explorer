// frontend/src/stores/navigation.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'

export type View = 'connections' | 'explorer'

export const useNavigationStore = defineStore('navigation', () => {
  const activeView = ref<View>('explorer')

  function setActiveView(view: View) {
    activeView.value = view
  }

  return {
    activeView,
    setActiveView,
  }
})
