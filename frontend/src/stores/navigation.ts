// frontend/src/stores/navigation.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'

export type View = 'connections' | 'explorer'

export const useNavigationStore = defineStore('navigation', () => {
  const activeView = ref<View>('explorer')
  const selectedExplorerConnectionId = ref<string | null>(null)

  function setActiveView(view: View) {
    activeView.value = view
  }

  function setSelectedExplorerConnectionId(connectionId: string | null) {
    selectedExplorerConnectionId.value = connectionId
  }

  return {
    activeView,
    setActiveView,
    selectedExplorerConnectionId,
    setSelectedExplorerConnectionId,
  }
})
