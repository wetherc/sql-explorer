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
          v-for="row of files.rows"
          :key="row.path"
          class="file-row"
          :style="{ paddingLeft: rowIndent(row.depth) }"
          role="treeitem"
          :aria-level="row.depth + 1"
          :aria-expanded="row.kind === 'folder' ? files.openPaths.has(row.path) : undefined"
          tabindex="0"
          data-test="file-row"
          @click="activate(row)"
          @keydown.enter.prevent="activate(row)"
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
