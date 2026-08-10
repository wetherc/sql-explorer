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

      <v-tooltip
        location="bottom"
        :text="settings.isDark ? 'Use the light theme' : 'Use the dark theme'"
      >
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

    <CommandPalette
      :open="ui.paletteOpen"
      :commands="commands"
      :apple="apple"
      @update:open="ui.setPaletteOpen"
    />

    <v-dialog
      :model-value="ui.keyboardHelpOpen"
      max-width="520"
      @update:model-value="ui.setKeyboardHelpOpen"
    >
      <v-card>
        <v-card-title class="text-subtitle-1">Keys</v-card-title>
        <v-card-text>
          <div
            v-for="command in commandsWithKeys"
            :key="command.id"
            class="d-flex justify-space-between py-1"
            data-test="key-list-row"
          >
            <span>{{ command.title }}</span>
            <span class="text-medium-emphasis">{{ chordLabel(command.key as string, apple) }}</span>
          </div>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn text="Close" @click="ui.setKeyboardHelpOpen(false)" />
        </v-card-actions>
      </v-card>
    </v-dialog>

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
          <v-text-field
            :model-value="settings.settings.maxPinnedResults"
            label="Results a tab keeps"
            type="number"
            hint="The largest number of results one tab keeps against the next run."
            persistent-hint
            data-test="setting-max-pinned"
            @update:model-value="(value) => settings.update({ maxPinnedResults: Number(value) })"
          />
          <v-text-field
            :model-value="settings.settings.exportRowLimit"
            label="Export row limit"
            type="number"
            hint="The row limit of an export that writes straight to a file."
            persistent-hint
            data-test="setting-export-limit"
            @update:model-value="(value) => settings.update({ exportRowLimit: Number(value) })"
          />
          <v-text-field
            :model-value="settings.settings.athenaPricePerTerabyte"
            label="Athena price for each terabyte"
            type="number"
            step="0.01"
            prefix="$"
            hint="An estimate. The rate changes by region and by contract."
            persistent-hint
            data-test="setting-athena-price"
            @update:model-value="
              (value) => settings.update({ athenaPricePerTerabyte: Number(value) })
            "
          />
          <v-text-field
            :model-value="settings.settings.athenaScanWarningGb"
            label="Warn above this scan in gigabytes"
            type="number"
            hint="A statement that scans more than this raises a warning."
            persistent-hint
            data-test="setting-athena-warning"
            @update:model-value="(value) => settings.update({ athenaScanWarningGb: Number(value) })"
          />
          <v-text-field
            :model-value="settings.settings.schemaSnapshotColumns"
            label="Columns the editor learns"
            type="number"
            hint="The largest number of columns one read of a schema keeps."
            persistent-hint
            data-test="setting-snapshot-columns"
            @update:model-value="
              (value) => settings.update({ schemaSnapshotColumns: Number(value) })
            "
          />
          <v-switch
            :model-value="settings.settings.schemaSnapshotOwnConnection"
            color="primary"
            label="Read the schema on a second connection"
            hint="One more session on the server, and no wait for a statement of the user."
            persistent-hint
            data-test="setting-snapshot-connection"
            @update:model-value="
              (value) => settings.update({ schemaSnapshotOwnConnection: value === true })
            "
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
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useTheme } from 'vuetify'
import CommandPalette from '@/components/CommandPalette.vue'
import ConnectionManager from '@/components/ConnectionManager.vue'
import DbExplorer from '@/components/DbExplorer.vue'
import HistoryPanel from '@/components/HistoryPanel.vue'
import NoticeHost from '@/components/NoticeHost.vue'
import QueryTabs from '@/components/QueryTabs.vue'
import StatusBar from '@/components/StatusBar.vue'
import { api } from '@/lib/api'
import {
  chordLabel,
  commandEnabled,
  commandForEvent,
  tabActions,
  type Command,
} from '@/lib/commands'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useHistoryStore } from '@/stores/history'
import { useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import type { UnlistenFn } from '@tauri-apps/api/event'

type Panel = 'connections' | 'explorer' | 'history'

const connections = useConnectionsStore()
const explorer = useExplorerStore()
const history = useHistoryStore()
const settings = useSettingsStore()
const tabs = useTabsStore()
const ui = useUiStore()
const theme = useTheme()

/** True on macOS, where the key list names Cmd in place of Ctrl. */
const apple = /mac|iphone|ipad/i.test(navigator.userAgent)

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

/** The actions of the tab that is open, when a tab is open. */
function actionsOfActiveTab() {
  return tabActions(tabs.activeTabId)
}

function hasActiveTab(): boolean {
  return tabs.activeTabId !== null
}

function closeActiveTab(): void {
  if (tabs.activeTabId) {
    tabs.close(tabs.activeTabId)
  }
}

/**
 * Every command of the application. The key handler below reads this list,
 * and so does the palette, so a new command needs one record here.
 */
const commands: Command[] = [
  {
    id: 'query.run',
    title: 'Run the statement',
    group: 'Query',
    key: 'mod+enter',
    enabled: hasActiveTab,
    run: () => actionsOfActiveTab()?.runStatement(),
  },
  {
    id: 'query.runAll',
    title: 'Run the whole script',
    group: 'Query',
    key: 'mod+shift+enter',
    enabled: hasActiveTab,
    run: () => actionsOfActiveTab()?.runAll(),
  },
  {
    id: 'query.stop',
    title: 'Stop the statement',
    group: 'Query',
    key: 'mod+shift+c',
    enabled: hasActiveTab,
    run: () => actionsOfActiveTab()?.cancel(),
  },
  {
    id: 'editor.format',
    title: 'Format the statement',
    group: 'Editor',
    key: 'shift+alt+f',
    enabled: hasActiveTab,
    run: () => actionsOfActiveTab()?.format(),
  },
  {
    id: 'tab.new',
    title: 'New tab',
    group: 'Tabs',
    key: 'mod+t',
    run: () => tabs.add(),
  },
  {
    id: 'tab.close',
    title: 'Close the tab',
    group: 'Tabs',
    key: 'mod+w',
    enabled: hasActiveTab,
    run: closeActiveTab,
  },
  {
    id: 'view.connections',
    title: 'Show the connections',
    group: 'View',
    key: 'mod+1',
    run: () => {
      panel.value = 'connections'
    },
  },
  {
    id: 'view.explorer',
    title: 'Show the explorer',
    group: 'View',
    key: 'mod+2',
    run: () => {
      panel.value = 'explorer'
    },
  },
  {
    id: 'view.history',
    title: 'Show the history',
    group: 'View',
    key: 'mod+3',
    run: () => {
      panel.value = 'history'
    },
  },
  {
    id: 'app.settings',
    title: 'Open the settings',
    group: 'Application',
    key: 'mod+,',
    run: () => {
      settingsOpen.value = true
    },
  },
  {
    id: 'app.palette',
    title: 'Open the command palette',
    group: 'Application',
    key: 'mod+shift+p',
    run: () => ui.setPaletteOpen(true),
  },
  {
    id: 'app.keys',
    title: 'Show the key list',
    group: 'Application',
    key: 'f1',
    run: () => ui.setKeyboardHelpOpen(true),
  },
]

const commandsWithKeys = computed(() => commands.filter((command) => command.key !== null))

/**
 * True while a dialog stands open. A key of the application must not reach
 * through a dialog, because the dialog holds the attention of the user.
 */
function dialogIsOpen(): boolean {
  // The ARIA role marks a dialog whatever the component library names its
  // classes, and the class covers a Vuetify dialog that has not set the
  // role yet.
  return document.querySelector('[role="dialog"], .v-dialog.v-overlay--active') !== null
}

function onKeyDown(event: KeyboardEvent): void {
  if (dialogIsOpen()) {
    return
  }
  const command = commandForEvent(commands, event)
  if (!command || !commandEnabled(command)) {
    return
  }
  // The host window binds some of these keys itself, so the event must not
  // travel any further.
  event.preventDefault()
  command.run()
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
  window.addEventListener('keydown', onKeyDown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeyDown)
  unlisten?.()
  unlisten = null
})

watch(
  () => settings.settings.theme,
  (name) => theme.change(name),
)

// The open tabs are written back when they change, so a restart finds the
// same workspace. The write waits for a short pause, because a keystroke in
// the editor changes the tabs and one write for each keystroke would put a
// file write behind every letter.
const PERSIST_DELAY_MS = 250
let persistTimer: ReturnType<typeof setTimeout> | null = null
watch(
  () => JSON.stringify(tabs.snapshot()),
  () => {
    if (persistTimer !== null) {
      clearTimeout(persistTimer)
    }
    persistTimer = setTimeout(() => {
      persistTimer = null
      void tabs.persist()
    }, PERSIST_DELAY_MS)
  },
)
onBeforeUnmount(() => {
  if (persistTimer !== null) {
    clearTimeout(persistTimer)
    persistTimer = null
    void tabs.persist()
  }
})
</script>

<style scoped>
.app-title {
  font-size: var(--app-text-lg);
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
