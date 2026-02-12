<template>
  <v-list dense>
    <template v-for="node in nodes" :key="node.key">
      <v-list-group v-if="node.children">
        <template v-slot:activator="{ props }">
          <v-list-item
            v-bind="props"
            :prepend-icon="node.icon || 'mdi-folder'"
            :title="node.label"
            @click.stop="$emit('expand', node)"
            @contextmenu.prevent="$emit('contextmenu', { event: $event, node })"
          ></v-list-item>
        </template>
        <AppTreeview 
          :nodes="node.children" 
          @expand="(node) => $emit('expand', node)" 
          @node-click="(node) => $emit('node-click', node)"
          @contextmenu="(data) => $emit('contextmenu', data)" 
        />
      </v-list-group>
      <v-list-item
        v-else
        :prepend-icon="node.icon || 'mdi-file-document-outline'"
        :title="node.label"
        @click="$emit('node-click', node)"
        @contextmenu.prevent="$emit('contextmenu', { event: $event, node })"
      ></v-list-item>
    </template>
  </v-list>
</template>

<script setup lang="ts">
import AppTreeview from './AppTreeview.vue'
import type { ExplorerNode } from '@/stores/explorer'

defineProps<{
  nodes: ExplorerNode[]
}>()

defineEmits<{
  (e: 'node-click', node: ExplorerNode): void
  (e: 'contextmenu', data: { event: MouseEvent, node: ExplorerNode }): void
  (e: 'expand', node: ExplorerNode): void
}>()
</script>

<style scoped>
/* Scoped styles for the treeview if needed */
</style>
