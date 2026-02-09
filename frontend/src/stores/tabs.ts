import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { v4 as uuidv4 } from 'uuid'

export interface QueryResult {
  columns: string[];
  rows: Record<string, any>[];
  errorMessage: string | null;
  isLoading: boolean;
}

export interface QueryTab {
  id: string;
  title: string;
  query: string;
  results: QueryResult;
  isSaved: boolean; // Future use for saving queries
}

export const useTabsStore = defineStore('tabs', () => {
  const tabs = ref<QueryTab[]>([])
  const activeTabId = ref<string | null>(null)

  const activeTab = computed(() => {
    return tabs.value.find(tab => tab.id === activeTabId.value) || null
  })

  function addTab(queryContent: string = '') {
    const newTab: QueryTab = {
      id: uuidv4(),
      title: `New Query ${tabs.value.length + 1}`,
      query: queryContent,
      results: {
        columns: [],
        rows: [],
        errorMessage: null,
        isLoading: false,
      },
      isSaved: false,
    }
    tabs.value.push(newTab)
    activeTabId.value = newTab.id
  }

  function closeTab(id: string) {
    const index = tabs.value.findIndex(tab => tab.id === id)
    if (index !== -1) {
      tabs.value.splice(index, 1)

      // If the closed tab was active, activate another tab
      if (activeTabId.value === id) {
        if (tabs.value.length > 0) {
          activeTabId.value = tabs.value[Math.max(0, index - 1)].id
        } else {
          activeTabId.value = null
        }
      }
    }
  }

  function setActiveTab(id: string) {
    activeTabId.value = id
  }

  return {
    tabs,
    activeTabId,
    activeTab,
    addTab,
    closeTab,
    setActiveTab,
  }
})
