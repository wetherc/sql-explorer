<template>
  <v-app>
    <v-app-bar density="compact" flat border="b">
      <v-app-bar-title class="app-title">
        <v-icon size="small" class="mr-2">mdi-database-search</v-icon>
        SQL Explorer
      </v-app-bar-title>

      <v-btn
        size="small"
        prepend-icon="mdi-plus"
        text="New query"
        data-test="app-new-query"
        @click="tabs.add()"
      />

      <v-tooltip location="bottom" :text="settings.isDark ? 'Use the light theme' : 'Use the dark theme'">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            :icon="settings.isDark ? 'mdi-weather-sunny' : 'mdi-weather-night'"
            size="small"
            aria-label="Change the theme"
            data-test="theme-toggle"
            @click="settings.toggleTheme()"
          />
        </template>
      </v-tooltip>

      <v-tooltip location="bottom" text="Settings">
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            icon="mdi-cog-outline"
            size="small"
            aria-label="Settings"
            data-test="open-settings"
            @click="settingsOpen = true"
          />
        </template>
      </v-tooltip>
    </v-app-bar>

    <v-navigation-drawer permanent rail :width="56" class="rail">
      <v-list density="compact" nav>
        <v-tooltip v-for="item in railItems" :key="item.value" location="right" :text="item.label">
          <template #activator="{ props: tip }">
            <v-list-item
              v-bind="tip"
              :active="panel === item.value"
              :prepend-icon="item.icon"
              :aria-label="item.label"
              :data-test="`rail-${item.value}`"
              @click="panel = item.value"
            />
          </template>
        </v-tooltip>
      </v-list>
    </v-navigation-drawer>

    <v-navigation-drawer permanent :width="320" class="side-panel">
      <ConnectionManager v-show="panel === 'connections'" @connected="onConnected" />
      <DbExplorer v-show="panel === 'explorer'" @open-connections="panel = 'connections'" />
      <HistoryPanel v-show="panel === 'history'" />
    </v-navigation-drawer>

    <v-main class="main-area">
      <div class="main-content">
        <QueryTabs @open-connections="panel = 'connections'" />
        <StatusBar />
      </div>
    </v-main>

    <NoticeHost />

    <v-dialog v-model="settingsOpen" max-width="520">
      <v-card>
        <v-card-title class="text-subtitle-1">Settings</v-card-title>
        <v-card-text class="d-flex flex-column ga-4">
          <v-slider
            :model-value="settings.settings.fontSize"
            label="Editor text size"
            :min="9"
            :max="24"
            :step="1"
            thumb-label
            hide-details
            data-test="setting-font-size"
            @update:model-value="(value) => settings.update({ fontSize: Number(value) })"
          />
          <v-switch
            :model-value="settings.settings.wordWrap"
            label="Wrap long lines"
            hide-details
            @update:model-value="(value) => settings.update({ wordWrap: Boolean(value) })"
          />
          <v-switch
            :model-value="settings.settings.showLineNumbers"
            label="Show line numbers"
            hide-details
            @update:model-value="(value) => settings.update({ showLineNumbers: Boolean(value) })"
          />
          <v-switch
            :model-value="settings.settings.autoRunPreview"
            label="Run a preview at once"
            hide-details
            @update:model-value="(value) => settings.update({ autoRunPreview: Boolean(value) })"
          />
          <v-text-field
            :model-value="settings.settings.maxRows"
            label="Row limit"
            type="number"
            hint="The largest number of rows one result set holds."
            persistent-hint
            data-test="setting-max-rows"
            @update:model-value="(value) => settings.update({ maxRows: Number(value) })"
          />
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Close" @click="settingsOpen = false" />
        </v-card-actions>
      </v-card>
    </v-dialog>
  </v-app>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useTheme } from 'vuetify'
import ConnectionManager from '@/components/ConnectionManager.vue'
import DbExplorer from '@/components/DbExplorer.vue'
import HistoryPanel from '@/components/HistoryPanel.vue'
import NoticeHost from '@/components/NoticeHost.vue'
import QueryTabs from '@/components/QueryTabs.vue'
import StatusBar from '@/components/StatusBar.vue'
import { api } from '@/lib/api'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useHistoryStore } from '@/stores/history'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import type { UnlistenFn } from '@tauri-apps/api/event'

type Panel = 'connections' | 'explorer' | 'history'

const connections = useConnectionsStore()
const explorer = useExplorerStore()
const history = useHistoryStore()
const settings = useSettingsStore()
const tabs = useTabsStore()
const theme = useTheme()

const panel = ref<Panel>('connections')
const settingsOpen = ref(false)
let unlisten: UnlistenFn | null = null

const railItems: Array<{ value: Panel; icon: string; label: string }> = [
  { value: 'connections', icon: 'mdi-lan-connect', label: 'Connections' },
  { value: 'explorer', icon: 'mdi-database-search', label: 'Explorer' },
  { value: 'history', icon: 'mdi-history', label: 'History and saved statements' },
]

function onConnected(): void {
  panel.value = 'explorer'
}

onMounted(async () => {
  settings.load()
  theme.change(settings.settings.theme)
  await connections.loadEngines()
  await connections.load()
  await history.load()
  await tabs.restore()
  for (const info of Object.values(connections.active)) {
    explorer.addRoot(info.connectionId)
  }
  unlisten = await api.onConnectionStatus((event) => connections.applyStatus(event))
})

onBeforeUnmount(() => {
  unlisten?.()
  unlisten = null
})

watch(
  () => settings.settings.theme,
  (name) => theme.change(name),
)

// The open tabs are written back whenever they change, so a restart finds
// the same workspace.
watch(
  () => tabs.snapshot(),
  () => {
    void tabs.persist()
  },
  { deep: true },
)
</script>

<style scoped>
.app-title {
  font-size: 0.9375rem;
  font-weight: 600;
}

.rail :deep(.v-list-item) {
  justify-content: center;
}

.main-area {
  height: 100%;
}

.main-content {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.main-content > :first-child {
  flex: 1 1 auto;
  min-height: 0;
}
</style>
