<template>
  <v-app>
    <v-app-bar density="compact" flat border="b">
      <v-app-bar-title class="app-title">
        <v-icon size="small" class="mr-2">mdi-database-search</v-icon>
        SQL Explorer
      </v-app-bar-title>

      <v-tooltip
        location="bottom"
        :text="settings.isDark ? 'Switch to light theme' : 'Switch to dark theme'"
      >
        <template #activator="{ props: tip }">
          <v-btn
            v-bind="tip"
            :icon="settings.isDark ? 'mdi-weather-sunny' : 'mdi-weather-night'"
            size="small"
            :aria-label="settings.isDark ? 'Switch to light theme' : 'Switch to dark theme'"
            data-test="theme-toggle"
            @click="settings.toggleTheme()"
          />
        </template>
      </v-tooltip>

      <v-tooltip location="bottom" :text="`Settings (${chordLabel('mod+,', apple)})`">
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
        <v-tooltip
          v-for="item in railItems"
          :key="item.value"
          location="right"
          :text="`${item.label} (${chordLabel(item.key, apple)})`"
        >
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

      <!-- The guide opens a dialog and joins no panel, because the side
           panel is too narrow for prose. -->
      <template #append>
        <v-list density="compact" nav>
          <v-tooltip location="right" text="Guide">
            <template #activator="{ props: tip }">
              <v-list-item
                v-bind="tip"
                prepend-icon="mdi-help-circle-outline"
                aria-label="Guide"
                data-test="rail-guide"
                @click="ui.setGuideOpen(true)"
              />
            </template>
          </v-tooltip>
        </v-list>
      </template>
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
      <FilesPanel v-show="layout.layout.panel === 'files'" />
      <HistoryPanel v-show="layout.layout.panel === 'history'" />

      <!-- The edge of the panel answers a drag and an arrow key. The ARIA
           role of a separator with a value is what a screen reader reads as
           a window splitter it can move. -->
      <div
        class="panel-resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="Side panel width"
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
        <QueryTabs ref="queryTabs" @open-connections="layout.showPanel('connections')" />
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

    <GuideDialog :open="ui.guideOpen" @update:open="ui.setGuideOpen" />

    <AppDialog
      :model-value="ui.keyboardHelpOpen"
      size="small"
      @update:model-value="ui.setKeyboardHelpOpen"
    >
      <v-card>
        <v-card-title>Keyboard shortcuts</v-card-title>
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

    <SettingsDialog v-model:open="settingsOpen" />
  </v-app>
</template>

<script setup lang="ts">
import AppDialog from '@/components/AppDialog.vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useTheme } from 'vuetify'
import CommandPalette from '@/components/CommandPalette.vue'
import ConnectionManager from '@/components/ConnectionManager.vue'
import GuideDialog from '@/components/GuideDialog.vue'
import DbExplorer from '@/components/DbExplorer.vue'
import FilesPanel from '@/components/FilesPanel.vue'
import HistoryPanel from '@/components/HistoryPanel.vue'
import NoticeHost from '@/components/NoticeHost.vue'
import QueryTabs from '@/components/QueryTabs.vue'
import SettingsDialog from '@/components/SettingsDialog.vue'
import StatusBar from '@/components/StatusBar.vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api } from '@/lib/api'
import {
  chordLabel,
  commandEnabled,
  appleKeyboard,
  commandForEvent,
  tabActions,
  type Command,
} from '@/lib/commands'
import { holdBackHostMenu } from '@/lib/contextmenu'
import { useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useFilesStore } from '@/stores/files'
import { useHistoryStore } from '@/stores/history'
import {
  MAX_PANEL_WIDTH,
  MIN_PANEL_WIDTH,
  PANEL_WIDTH_STEP,
  useLayoutStore,
  type Panel,
} from '@/stores/layout'
import { useSettingsStore } from '@/stores/settings'
import { useQueryStore } from '@/stores/query'
import { useTabsStore } from '@/stores/tabs'
import { useUiStore } from '@/stores/ui'
import type { UnlistenFn } from '@tauri-apps/api/event'

const connections = useConnectionsStore()
const explorer = useExplorerStore()
const files = useFilesStore()
const history = useHistoryStore()
const layout = useLayoutStore()
const settings = useSettingsStore()
const tabs = useTabsStore()
const queries = useQueryStore()
const ui = useUiStore()
const theme = useTheme()

/** The width of the rail of icons, which a drag of the panel edge allows for. */
const RAIL_WIDTH = 56

/** True on macOS, where the key list names Cmd in place of Ctrl. */
const apple = appleKeyboard()

const settingsOpen = ref(false)
let unlisten: UnlistenFn | null = null
/** The listener of the menu of the operating system. */
let unlistenMenu: UnlistenFn | null = null
/** The listener of the request of the window to close. */
let unlistenClose: UnlistenFn | null = null
/** Stops holding back the menu of the host, once the shell holds it back. */
let unholdHostMenu: (() => void) | null = null

const railItems: Array<{ value: Panel; icon: string; label: string; key: string }> = [
  { value: 'connections', icon: 'mdi-lan-connect', label: 'Connections', key: 'mod+1' },
  { value: 'explorer', icon: 'mdi-database-search', label: 'Explorer', key: 'mod+2' },
  { value: 'files', icon: 'mdi-folder-outline', label: 'Files', key: 'mod+3' },
  { value: 'history', icon: 'mdi-history', label: 'History', key: 'mod+4' },
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

/** The tab row, which holds the edit of the name of a tab. */
const queryTabs = ref<InstanceType<typeof QueryTabs> | null>(null)

/** The actions of the tab that is open, when a tab is open. */
function actionsOfActiveTab() {
  return tabActions(tabs.activeTabId)
}

function hasActiveTab(): boolean {
  return tabs.activeTabId !== null
}

/** The reason a command of a tab can't run while no tab is open. */
const NO_TAB_REASON = 'Open a query tab first.'

/** True while a statement of the open tab runs. */
function isRunning(): boolean {
  const id = tabs.activeTabId
  return id !== null && queries.peekState(id)?.running === true
}

/**
 * Every command of the application. The key handler below reads this list,
 * and so does the palette, so a new command needs one record here.
 */
const commands: Command[] = [
  {
    id: 'query.run',
    title: 'Run statement',
    group: 'Query',
    key: 'mod+enter',
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => actionsOfActiveTab()?.runStatement(),
  },
  {
    id: 'query.runAll',
    title: 'Run script',
    group: 'Query',
    key: 'mod+shift+enter',
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => actionsOfActiveTab()?.runAll(),
  },
  {
    id: 'query.stop',
    title: 'Stop',
    group: 'Query',
    key: 'mod+shift+c',
    enabled: isRunning,
    disabledReason: () => (hasActiveTab() ? 'Nothing is running.' : NO_TAB_REASON),
    run: () => actionsOfActiveTab()?.cancel(),
  },
  {
    id: 'query.save',
    title: 'Save to file',
    group: 'File',
    key: 'mod+s',
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => actionsOfActiveTab()?.save(),
  },
  {
    id: 'editor.format',
    title: 'Format SQL',
    group: 'Editor',
    key: 'shift+alt+f',
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => actionsOfActiveTab()?.format(),
  },
  {
    id: 'tab.new',
    title: 'New query',
    group: 'File',
    key: 'mod+t',
    // The desktop names this key Ctrl+N, and the editors this application
    // grew beside name it Ctrl+T, so both reach it.
    aliases: ['mod+n'],
    run: () => tabs.add(),
  },
  {
    id: 'file.open',
    title: 'Open query…',
    group: 'File',
    key: 'mod+o',
    run: () => void files.openFileFromDialog(),
  },
  {
    id: 'file.openFolder',
    title: 'Open folder…',
    group: 'File',
    key: 'mod+shift+o',
    run: () => void files.openFolder(),
  },
  {
    id: 'tab.rename',
    title: 'Rename tab',
    group: 'Tabs',
    key: null,
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => queryTabs.value?.renameActiveTab(),
  },
  {
    id: 'tab.close',
    title: 'Close tab',
    group: 'Tabs',
    key: 'mod+w',
    enabled: hasActiveTab,
    disabledReason: () => NO_TAB_REASON,
    run: () => queryTabs.value?.closeActiveTab(),
  },
  {
    id: 'view.connections',
    title: 'Show connections',
    group: 'View',
    key: 'mod+1',
    run: () => layout.showPanel('connections'),
  },
  {
    id: 'view.explorer',
    title: 'Show explorer',
    group: 'View',
    key: 'mod+2',
    run: () => layout.showPanel('explorer'),
  },
  {
    id: 'view.files',
    title: 'Show files',
    group: 'View',
    key: 'mod+3',
    run: () => layout.showPanel('files'),
  },
  {
    id: 'view.history',
    title: 'Show history',
    group: 'View',
    key: 'mod+4',
    run: () => layout.showPanel('history'),
  },
  {
    id: 'view.togglePanel',
    title: 'Toggle side panel',
    group: 'View',
    key: 'mod+b',
    run: () => layout.togglePanel(),
  },
  {
    id: 'view.results',
    title: 'Toggle results panel',
    group: 'View',
    key: 'mod+j',
    run: () => layout.toggleResults(),
  },
  {
    id: 'app.settings',
    title: 'Open settings',
    group: 'Application',
    key: 'mod+,',
    run: () => {
      settingsOpen.value = true
    },
  },
  {
    id: 'app.palette',
    title: 'Command palette',
    group: 'Application',
    key: 'mod+shift+p',
    run: () => ui.setPaletteOpen(true),
  },
  {
    id: 'app.keys',
    title: 'Keyboard shortcuts',
    group: 'Application',
    key: 'f1',
    run: () => ui.setKeyboardHelpOpen(true),
  },
  {
    id: 'app.guide',
    title: 'Open guide',
    group: 'Application',
    key: null,
    run: () => ui.setGuideOpen(true),
  },
]

const commandsWithKeys = computed(() => commands.filter((command) => command.key !== null))

/**
 * Runs the command that an identifier names. The menu of the operating
 * system sends such an identifier, so a command reached from the menu and a
 * command reached from a key follow one path.
 */
function runCommandById(id: string): void {
  // A command from the menu must not act behind a dialog, for the same
  // reason as a key.
  if (ui.dialogOpen) {
    return
  }
  const command = commands.find((entry) => entry.id === id)
  if (command && commandEnabled(command)) {
    command.run()
  }
}

/** The commands that the menu of the operating system draws. */
const MENU_COMMAND_IDS: string[] = ['tab.new', 'file.open', 'file.openFolder', 'query.save']

/**
 * What the menu of the operating system shows for each command it holds.
 * The state of a command lives here, because the tabs and the connections
 * that decide it live here, so the backend is told of each change.
 */
const menuCommandStates = computed(() =>
  // The order follows the menu and not the registry, so a reader of the
  // two lists sees one order.
  MENU_COMMAND_IDS.map((id) => {
    const command = commands.find((entry) => entry.id === id)
    return { id, enabled: command ? !ui.dialogOpen && commandEnabled(command) : false }
  }),
)

watch(
  menuCommandStates,
  (states) => {
    // A menu that keeps an old state is a menu that reads wrongly, and
    // nothing else fails with it, so the failure stays out of the notices.
    void Promise.resolve(api.setMenuCommands(states)).catch(() => {})
  },
  { immediate: true, deep: true },
)

function onKeyDown(event: KeyboardEvent): void {
  // A key of the application must not reach through a dialog, because the
  // dialog holds the attention of the user. Each dialog counts itself in the
  // store as it opens, so the shell asks the store and not the document.
  if (ui.dialogOpen) {
    return
  }
  const command = commandForEvent(commands, event, apple)
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
  // The keys are bound before anything is read. A read that fails would
  // otherwise take the binding with it, and the user would then have an
  // application that answers no key at all.
  window.addEventListener('keydown', onKeyDown)
  unholdHostMenu = holdBackHostMenu(window)

  await connections.loadEngines()
  await connections.load()
  await history.load()
  // The backend records the folders and the files that the user accepted,
  // so the panel reads that record and shows those folders alone. The read
  // also lets the backend admit those paths, so it comes before the tabs
  // compare their files with the disk.
  await files.restoreRoots()
  await tabs.restore()
  for (const info of Object.values(connections.active)) {
    explorer.addRoot(info.connectionId)
  }
  try {
    unlistenMenu = await api.onMenuCommand(runCommandById)
  } catch (error) {
    // The window still answers every key without the menu of the system.
    ui.reportError(error)
  }
  await showStorageProblems()
  try {
    unlistenClose = await getCurrentWindow().onCloseRequested(flushPersist)
  } catch {
    // Outside the desktop host there is no window to close, and the write
    // after the pause still keeps the tabs.
  }
  try {
    unlisten = await api.onConnectionStatus((event) => connections.applyStatus(event))
  } catch (error) {
    // The application still runs without the reports of the backend. It then
    // learns of a connection that dropped when it next uses that connection.
    ui.reportError(error)
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeyDown)
  unholdHostMenu?.()
  // A drag that is still under way would otherwise leave its listeners and the
  // class it put on the body behind.
  endPanelDrag()
  unwatchSystemTheme()
  unlisten?.()
  unlisten = null
  unlistenMenu?.()
  unlistenMenu = null
  unlistenClose?.()
  unlistenClose = null
})

/**
 * Shows each problem the backend found with its stored files, such as a
 * settings file it could not read. Each one stays until the user takes it
 * away, because the user may need to act on it. A failed read reports
 * nothing.
 */
async function showStorageProblems(): Promise<void> {
  let problems: string[] = []
  try {
    problems = (await api.storageProblems()) ?? []
  } catch {
    return
  }
  for (const problem of problems) {
    ui.warn(problem, null, { kept: true })
  }
}

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
//
// The watch follows one count that the store raises on each change. It
// therefore walks no tab record and serialises no statement while the user
// writes. The records serialise once, inside the write after the pause.
const PERSIST_DELAY_MS = 250
let persistTimer: ReturnType<typeof setTimeout> | null = null
watch(
  () => tabs.revision,
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

/**
 * Writes the tabs at once when a write waits for its pause. The window waits
 * for this before it closes, so the last keystrokes reach the disk.
 */
async function flushPersist(): Promise<void> {
  if (persistTimer === null) {
    return
  }
  clearTimeout(persistTimer)
  persistTimer = null
  await tabs.persist()
}

onBeforeUnmount(() => {
  void flushPersist()
})
</script>

<style scoped>
.app-title {
  font-size: var(--app-text-lg);
  font-weight: 600;
}

/* The buttons at the end of the bar keep a gap from the edge of the window
   and from each other, as the buttons of the panel headers do. */
.v-app-bar :deep(.v-toolbar__content) {
  gap: 4px;
  padding-inline-end: 10px;
}

/* A row of the rail has an icon and no text. The library keeps a gap of 32
   pixels after the icon for the text that would follow it, and the icon with
   that gap is wider than the row, so the icon would sit against the edge of
   the window and outside the highlight behind it. Without the gap the icon
   sits in the middle of its row. */
.rail :deep(.v-list-item) {
  --v-list-prepend-gap: 0px;

  justify-content: center;
}

/* The icons of the rail are one step larger than the icons of a toolbar,
   because they are the only mark on each row. */
.rail :deep(.v-list-item .v-icon) {
  font-size: 20px;
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
