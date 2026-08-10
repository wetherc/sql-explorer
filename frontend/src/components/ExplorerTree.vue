<template>
  <ul class="explorer-tree" role="group">
    <li v-for="node in nodes" :key="node.key" role="treeitem" :aria-expanded="ariaExpanded(node)">
      <div
        class="tree-row"
        :class="{ selected: selectedKey === node.key }"
        :style="{ paddingLeft: `${depth * 14 + 6}px` }"
        tabindex="0"
        data-test="tree-row"
        @click="onActivate(node)"
        @keydown.enter.prevent="onActivate(node)"
        @keydown.space.prevent="onActivate(node)"
        @contextmenu.prevent="emit('context', { event: $event, node })"
      >
        <v-icon v-if="canExpand(node)" size="x-small" class="chevron" data-test="tree-chevron">
          {{ isOpen(node) ? 'mdi-chevron-down' : 'mdi-chevron-right' }}
        </v-icon>
        <span v-else class="chevron-space"></span>

        <v-progress-circular
          v-if="node.loading"
          indeterminate
          size="12"
          width="2"
          class="mr-2"
          data-test="tree-loading"
        />
        <v-icon v-else size="small" class="mr-2 node-icon">{{ node.icon }}</v-icon>

        <span class="node-label">{{ node.label }}</span>
        <span v-if="node.hint" class="node-hint">{{ node.hint }}</span>
      </div>

      <ExplorerTree
        v-if="node.children && isOpen(node)"
        :nodes="node.children"
        :depth="depth + 1"
        :open-keys="openKeys"
        :selected-key="selectedKey"
        @activate="(child) => emit('activate', child)"
        @context="(payload) => emit('context', payload)"
      />

      <div
        v-if="isOpen(node) && node.loaded && (node.children?.length ?? 0) === 0"
        class="empty-branch"
        :style="{ paddingLeft: `${(depth + 1) * 14 + 24}px` }"
      >
        Nothing here
      </div>
    </li>
  </ul>
</template>

<script setup lang="ts">
import { isExpandable, type ExplorerNode } from '@/stores/explorer'

const props = withDefaults(
  defineProps<{
    nodes: ExplorerNode[]
    depth?: number
    openKeys: Set<string>
    selectedKey?: string | null
  }>(),
  { depth: 0, selectedKey: null },
)

const emit = defineEmits<{
  (event: 'activate', node: ExplorerNode): void
  (event: 'context', payload: { event: MouseEvent; node: ExplorerNode }): void
}>()

function canExpand(node: ExplorerNode): boolean {
  return isExpandable(node)
}

function isOpen(node: ExplorerNode): boolean {
  return props.openKeys.has(node.key)
}

function ariaExpanded(node: ExplorerNode): 'true' | 'false' | undefined {
  return canExpand(node) ? (isOpen(node) ? 'true' : 'false') : undefined
}

function onActivate(node: ExplorerNode): void {
  emit('activate', node)
}
</script>

<style scoped>
.explorer-tree {
  list-style: none;
  margin: 0;
  padding: 0;
}

.tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
  padding-top: 3px;
  padding-bottom: 3px;
  padding-right: 8px;
  cursor: pointer;
  font-size: var(--app-text-md);
  user-select: none;
}

.tree-row:hover {
  background: rgba(var(--v-theme-primary), 0.08);
}

.tree-row:focus-visible {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}

.tree-row.selected {
  background: rgba(var(--v-theme-primary), 0.16);
}

.chevron,
.chevron-space {
  width: 16px;
  flex: 0 0 16px;
}

.node-icon {
  color: rgb(var(--v-theme-on-surface-variant));
}

.node-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-hint {
  margin-left: 8px;
  font-size: var(--app-text-xs);
  color: rgb(var(--v-theme-on-surface-variant));
  white-space: nowrap;
}

.empty-branch {
  font-size: var(--app-text-sm);
  font-style: italic;
  color: rgb(var(--v-theme-on-surface-variant));
  padding-top: 2px;
  padding-bottom: 2px;
}
</style>
