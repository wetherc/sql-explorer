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

    <v-navigation-drawer permanent rail :width="RAIL_WIDTH" class="rail">
      <v-list density="compact" nav>
        <v-tooltip v-for="item in railItems" :key="item.value" location="right" :text="item.label">
          <template #activator="{ props: tip }">
            <v-list-item
              v-bind="tip"
              :active="layout.layout.panel === item.value"
              :prepend-icon="item.icon"
              :aria-label="item.label"
              :aria-expanded="layout.layout.panel === item.value && layout.layout.panelOpen"
              :data-test="`rail-${item.value}`"
              @click="layout.selectPanel(item.value)"
            />
          </template>
        </v-tooltip>
      </v-list>
    </v-navigation-drawer>

    <v-navigation-drawer
      :model-value="layout.layout.panelOpen"
      permanent
      :width="layout.layout.panelWidth"
      class="side-panel"
    >
      <ConnectionManager v-show="layout.layout.panel === 'connections'" @connected="onConnected" />
      <DbExplorer
        v-show="layout.layout.panel === 'explorer'"
        @open-connections="layout.showPanel('connections')"
      />
      <HistoryPanel v-show="layout.layout.panel === 'history'" />

      <!-- The edge of the panel answers a drag and an arrow key. The ARIA
           role of a separator with a value is what a screen reader reads as
           a window splitter it can move. -->
      <div
        class="panel-resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="Width of the side panel"
        :aria-valuenow="layout.layout.panelWidth"
        :aria-valuemin="MIN_PANEL_WIDTH"
        :aria-valuemax="MAX_PANEL_WIDTH"
        tabindex="0"
        data-test="panel-resizer"
        @pointerdown="startPanelDrag"
        @keydown.left.prevent="layout.nudgePanelWidth(-PANEL_WIDTH_STEP)"
        @keydown.right.prevent="layout.nudgePanelWidth(PANEL_WIDTH_STEP)"
        @keydown.home.prevent="layout.setPanelWidth(MIN_PANEL_WIDTH)"
        @keydown.end.prevent="layout.setPanelWidth(MAX_PANEL_WIDTH)"
      ></div>
    </v-navigation-drawer>

    <v-main class="main-area">
      <div class="main-content">
        <QueryTabs @open-connections="layout.showPanel('connections')" />
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

    <AppDialog
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
    </AppDialog>

    <AppDialog v-model="settingsOpen" max-width="520">
      <v-card>
        <v-card-title class="text-subtitle-1">Settings</v-card-title>
        <v-card-text class="d-flex flex-column ga-4">
          <v-select
            :model-value="settings.settings.theme"
            :items="THEME_CHOICES"
            label="Theme"
            data-test="setting-theme"
            @update:model-value="(value) => settings.update({ theme: value })"
          />
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
    </AppDialog>
  </v-app>
</template>

<script setup lang="ts">
import AppDialog from '@/components/AppDialog.vue'
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
import {
  MAX_PANEL_WIDTH,
  MIN_PANEL_WIDTH,
  PANEL_WIDTH_STEP,
  useLayoutStore,
  type Panel,
} from '@/stores/layout'
import { THEME_CHOICES, useSettingsStore } from '@/stores/settings'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import type { UnlistenFn } from '@tauri-apps/api/event'

const connections = useConnectionsStore()
const explorer = useExplorerStore()
const history = useHistoryStore()
const layout = useLayoutStore()
const settings = useSettingsStore()
const tabs = useTabsStore()
const ui = useUiStore()
const theme = useTheme()

/** The width of the rail of icons, which a drag of the panel edge allows for. */
const RAIL_WIDTH = 56

/** True on macOS, where the key list names Cmd in place of Ctrl. */
const apple = /mac|iphone|ipad/i.test(navigator.userAgent)

const settingsOpen = ref(false)
let unlisten: UnlistenFn | null = null

const railItems: Array<{ value: Panel; icon: string; label: string }> = [
  { value: 'connections', icon: 'mdi-lan-connect', label: 'Connections' },
  { value: 'explorer', icon: 'mdi-database-search', label: 'Explorer' },
  { value: 'history', icon: 'mdi-history', label: 'History and saved statements' },
]

function onConnected(): void {
  layout.showPanel('explorer')
}

/** The class that stops the drag from marking text under the pointer. */
const RESIZING_CLASS = 'app-resizing'

/** The place the pointer last reported, which the next frame reads. */
let pendingPanelWidth: number | null = null
let panelFrame: number | null = null

/**
 * Follows the pointer while it drags the edge of the side panel. The width is
 * the distance from the left of the window less the width of the rail, so the
 * edge stays under the pointer.
 *
 * A pointer reports its place more often than the screen draws, so each report
 * only holds the figure and the width changes once for each frame. The drag
 * also takes the pointer for itself, which keeps the events coming while the
 * pointer stands over the editor or the tree.
 */
function startPanelDrag(event: PointerEvent): void {
  // Without this the press starts a selection, and the text of the tree or the
  // editor then marks itself as the pointer crosses it.
  event.preventDefault()
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture?.(event.pointerId)
  document.body.classList.add(RESIZING_CLASS)
  layout.beginPanelResize()

  window.addEventListener('pointermove', onPanelDragMove)
  window.addEventListener('pointerup', endPanelDrag)
  window.addEventListener('pointercancel', endPanelDrag)
}

function onPanelDragMove(move: PointerEvent): void {
  pendingPanelWidth = move.clientX - RAIL_WIDTH
  if (panelFrame !== null) {
    return
  }
  panelFrame = requestAnimationFrame(() => {
    panelFrame = null
    if (pendingPanelWidth !== null) {
      layout.setPanelWidth(pendingPanelWidth)
      pendingPanelWidth = null
    }
  })
}

function endPanelDrag(): void {
  window.removeEventListener('pointermove', onPanelDragMove)
  window.removeEventListener('pointerup', endPanelDrag)
  window.removeEventListener('pointercancel', endPanelDrag)
  if (panelFrame !== null) {
    cancelAnimationFrame(panelFrame)
    panelFrame = null
  }
  // The last report of the pointer may still wait for its frame, so it is
  // taken here and the panel ends the drag where the pointer left it.
  if (pendingPanelWidth !== null) {
    layout.setPanelWidth(pendingPanelWidth)
    pendingPanelWidth = null
  }
  document.body.classList.remove(RESIZING_CLASS)
  // A stray mark can survive the drag, so it goes here.
  window.getSelection()?.removeAllRanges()
  layout.endPanelResize()
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
    run: () => layout.showPanel('connections'),
  },
  {
    id: 'view.explorer',
    title: 'Show the explorer',
    group: 'View',
    key: 'mod+2',
    run: () => layout.showPanel('explorer'),
  },
  {
    id: 'view.history',
    title: 'Show the history',
    group: 'View',
    key: 'mod+3',
    run: () => layout.showPanel('history'),
  },
  {
    id: 'view.togglePanel',
    title: 'Show or hide the side panel',
    group: 'View',
    key: 'mod+b',
    run: () => layout.togglePanel(),
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

function onKeyDown(event: KeyboardEvent): void {
  // A key of the application must not reach through a dialog, because the
  // dialog holds the attention of the user. Each dialog counts itself in the
  // store as it opens, so the shell asks the store and not the document.
  if (ui.dialogOpen) {
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

// The settings, the shape of the work area and the theme of the host are read
// here and not when the shell is mounted. All three decide what the first
// frame looks like, so a read after the mount would draw the shell once in the
// dark theme at the starting width and then draw it again.
settings.load()
layout.load()
const unwatchSystemTheme = settings.watchSystemTheme()
theme.change(settings.resolvedTheme)

onMounted(async () => {
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
  // A drag that is still under way would otherwise leave its listeners and the
  // class it put on the body behind.
  endPanelDrag()
  unwatchSystemTheme()
  unlisten?.()
  unlisten = null
})

// The theme follows the choice of the user, and the theme of the host as well
// while the choice is to follow the host.
watch(
  () => settings.resolvedTheme,
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

/* The strip sits over the right edge of the panel. It reaches past the edge on
   both sides, so the pointer finds it without a careful aim. */
.panel-resizer {
  position: absolute;
  top: 0;
  right: -3px;
  width: 7px;
  height: 100%;
  z-index: 1;
  cursor: col-resize;
  touch-action: none;
}

.panel-resizer:hover,
.panel-resizer:focus-visible {
  background: rgb(var(--v-theme-primary));
  outline: none;
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
