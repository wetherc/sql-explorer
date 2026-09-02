<template>
  <div class="files-panel">
    <PanelHeader>
      <template #actions>
        <v-tooltip location="bottom" text="Open a folder">
          <template #activator="{ props: tip }">
            <v-btn
              v-bind="tip"
              icon="mdi-folder-plus-outline"
              size="small"
              aria-label="Open a folder"
              data-test="files-open-folder"
              @click="files.openFolder()"
            />
          </template>
        </v-tooltip>
      </template>
    </PanelHeader>

    <v-progress-linear v-if="files.loading" indeterminate height="2" />

    <div class="files-body">
      <div v-if="files.hasRoots" class="files-tree" role="tree" aria-label="Files">
        <div
          v-for="(row, index) of files.rows"
          :key="row.path"
          :ref="(element) => keepRow(row.path, element)"
          class="file-row"
          :style="{ paddingLeft: rowIndent(row.depth) }"
          role="treeitem"
          :aria-level="row.depth + 1"
          :aria-expanded="row.kind === 'folder' ? files.openPaths.has(row.path) : undefined"
          :tabindex="row.path === tabStop ? 0 : -1"
          :aria-keyshortcuts="row.depth === 0 ? 'Delete' : undefined"
          data-test="file-row"
          @click="activate(row)"
          @focus="activePath = row.path"
          @keydown="onKeydown($event, row, index)"
        >
          <v-icon
            v-if="row.kind === 'folder'"
            size="x-small"
            class="chevron"
            aria-hidden="true"
            data-test="file-chevron"
          >
            {{ files.openPaths.has(row.path) ? 'mdi-chevron-down' : 'mdi-chevron-right' }}
          </v-icon>
          <span v-else class="chevron-space"></span>

          <v-progress-circular
            v-if="row.loading"
            indeterminate
            size="12"
            width="2"
            class="mr-2"
            data-test="file-loading"
          />
          <v-icon v-else size="small" class="mr-2 file-icon" aria-hidden="true">
            {{ row.kind === 'folder' ? 'mdi-folder-outline' : 'mdi-file-document-outline' }}
          </v-icon>

          <span class="file-label">{{ row.name }}</span>

          <!-- A root can be taken out of the panel again. The mark answers
               the mouse alone, because the row itself is the control that a
               key reaches. -->
          <v-icon
            v-if="row.depth === 0"
            size="x-small"
            class="ml-2 close-mark"
            aria-hidden="true"
            data-test="close-root"
            @click.stop="files.closeRoot(row.path)"
          >
            mdi-close
          </v-icon>
        </div>
      </div>

      <EmptyState
        v-else
        icon="mdi-folder-open-outline"
        title="No folders yet"
        hint="Open a folder to reach the statements that it holds."
      >
        <v-btn
          color="primary"
          variant="flat"
          size="small"
          prepend-icon="mdi-folder-plus-outline"
          text="Open a folder"
          data-test="files-empty-open"
          @click="files.openFolder()"
        />
      </EmptyState>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import EmptyState from './EmptyState.vue'
import PanelHeader from './PanelHeader.vue'
import { useFilesStore, type FileNode } from '@/stores/files'

/** The width of one step of the indent. */
const INDENT_STEP = 12
/** The indent the first level starts at. */
const BASE_INDENT = 8

const files = useFilesStore()

function rowIndent(depth: number): string {
  return `${BASE_INDENT + depth * INDENT_STEP}px`
}

/** A folder opens and closes. A file opens in a tab. */
function activate(row: FileNode): void {
  if (row.kind !== 'folder') {
    void files.openFile(row.path)
    return
  }
  if (files.openPaths.has(row.path)) {
    files.collapse(row.path)
    return
  }
  void files.expand(row.path)
}

/**
 * The row that the keys reach, and the elements of the rows.
 *
 * A tree carries one tab stop, and the arrows move inside it. The stop
 * follows the row the user last reached, and it falls back on the first row
 * when that row is gone.
 */
const activePath = ref<string | null>(null)
const rowElements = new Map<string, HTMLElement>()

const tabStop = computed(() => {
  const held = files.rows.find((row) => row.path === activePath.value)
  return held?.path ?? files.rows[0]?.path ?? null
})

/** Holds the element of one row, and forgets a row that is gone. */
function keepRow(path: string, element: unknown): void {
  if (element instanceof HTMLElement) {
    rowElements.set(path, element)
  } else {
    rowElements.delete(path)
  }
}

/** Moves the stop to one row and puts the focus on it. */
function focusRow(path: string | undefined): void {
  if (path === undefined) {
    return
  }
  activePath.value = path
  void nextTick(() => rowElements.get(path)?.focus())
}

/** The row that holds the given one, when there is one. */
function parentOf(index: number): FileNode | undefined {
  const row = files.rows[index]
  if (!row || row.depth === 0) {
    return undefined
  }
  return files.rows
    .slice(0, index)
    .reverse()
    .find((candidate) => candidate.depth < row.depth)
}

/**
 * The keys of a tree. The arrows walk the rows on show, the right arrow
 * opens a folder and steps into it, the left arrow closes a folder or steps
 * out of it, and Delete takes a root out of the panel.
 */
function onKeydown(event: KeyboardEvent, row: FileNode, index: number): void {
  const open = row.kind === 'folder' && files.openPaths.has(row.path)
  switch (event.key) {
    case 'ArrowDown':
      focusRow(files.rows[index + 1]?.path)
      break
    case 'ArrowUp':
      focusRow(files.rows[index - 1]?.path)
      break
    case 'Home':
      focusRow(files.rows[0]?.path)
      break
    case 'End':
      focusRow(files.rows[files.rows.length - 1]?.path)
      break
    case 'ArrowRight':
      if (row.kind === 'folder' && !open) {
        void files.expand(row.path)
      } else if (open) {
        focusRow(files.rows[index + 1]?.path)
      }
      break
    case 'ArrowLeft':
      if (open) {
        files.collapse(row.path)
      } else {
        focusRow(parentOf(index)?.path)
      }
      break
    case 'Enter':
    case ' ':
      activate(row)
      break
    case 'Delete':
      if (row.depth === 0) {
        files.closeRoot(row.path)
      }
      break
    default:
      return
  }
  event.preventDefault()
}
</script>

<style scoped>
.files-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.files-body {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
  padding-top: 4px;
}

/* Each row takes the width of the widest row, so a long name reaches past
   the panel and the scroll of the panel brings it into view. */
.files-tree {
  width: max-content;
  min-width: 100%;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px 8px 3px 0;
  cursor: pointer;
  font-size: var(--app-text-md);
  user-select: none;
}

.file-row:hover {
  background: rgba(var(--v-theme-primary), 0.08);
}

.file-row:focus-visible {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}

.chevron,
.chevron-space {
  width: 16px;
  flex: 0 0 16px;
}

.file-icon {
  color: rgb(var(--v-theme-on-surface-variant));
}

.file-label {
  white-space: nowrap;
}

.close-mark {
  border-radius: 50%;
  padding: 2px;
}

.close-mark:hover {
  background: rgba(var(--v-theme-on-surface), 0.12);
}
</style>
