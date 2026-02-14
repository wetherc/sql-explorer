// src/stores/tabs.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { useConnectionStore } from './connection'

export interface QueryTab {
  id: string
  title: string
  query: string
  connectionId: string
}

let tabIdCounter = 0

export const useTabsStore = defineStore('tabs', () => {
  const tabs = ref<QueryTab[]>([])
  const activeTabId = ref<string | null>(null)

  function addTab(connectionId: string, query: string = '') {
    const connectionStore = useConnectionStore()
    const connection = connectionStore.activeConnections[connectionId]
    const connectionName = connection ? connection.name : 'Unknown'

    const newId = `tab-${tabIdCounter++}`
    const newTab: QueryTab = {
      id: newId,
      title: `[${connectionName}] - Query ${tabIdCounter}`,
      query: query,
      connectionId: connectionId,
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

  function updateTabConnection(tabId: string, newConnectionId: string) {
    const tab = tabs.value.find(t => t.id === tabId)
    if (tab) {
      const connectionStore = useConnectionStore()
      const connection = connectionStore.activeConnections[newConnectionId]
      const connectionName = connection ? connection.name : 'Unknown'
      
      tab.connectionId = newConnectionId
      // Keep the query number consistent
      const oldTitle = tab.title
      const queryNumberMatch = oldTitle.match(/Query (\d+)/)
      const queryNumber = queryNumberMatch ? queryNumberMatch[1] : tabIdCounter
      tab.title = `[${connectionName}] - Query ${queryNumber}`
    }
  }

  return {
    tabs,
    activeTabId,
    addTab,
    closeTab,
    setActiveTab,
    updateTabConnection,
  }
})
