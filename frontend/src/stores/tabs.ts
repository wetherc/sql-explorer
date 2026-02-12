// src/stores/tabs.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'

export interface QueryTab {
  id: string
  title: string
  query: string
}

let tabIdCounter = 0

export const useTabsStore = defineStore('tabs', () => {
  const tabs = ref<QueryTab[]>([])
  const activeTabId = ref<string | null>(null)

  function addTab(query: string = '') {
    const newId = `tab-${tabIdCounter++}`
    const newTab: QueryTab = {
      id: newId,
      title: `Query ${tabIdCounter}`,
      query: query,
    }
    tabs.value.push(newTab)
    activeTabId.value = newId
  }

  function closeTab(id: string) {
    const index = tabs.value.findIndex(t => t.id === id)
    if (index === -1) return

    tabs.value.splice(index, 1)

    // If the closed tab was the active one, set a new active tab
    if (activeTabId.value === id) {
      if (tabs.value.length > 0) {
        // Activate the previous tab or the first one
        const newActiveIndex = Math.max(0, index - 1)
        const newActiveTab = tabs.value[newActiveIndex]
        if (newActiveTab) {
          activeTabId.value = newActiveTab.id
        }
      } else {
        activeTabId.value = null
      }
    }
  }

  function setActiveTab(id: string) {
    activeTabId.value = id
  }

  return {
    tabs,
    activeTabId,
    addTab,
    closeTab,
    setActiveTab,
  }
})
