<template>
  <v-list dense density="compact">
    <template v-for="node in nodes" :key="node.key">
      <!-- Special rendering for Connection Root Node -->
      <v-list-group v-if="node.data.type === 'connection'">
        <template v-slot:activator="{ props }">
          <v-list-item
            v-bind="props"
            :title="node.label"
            :prepend-icon="node.icon"
            class="connection-banner"
          ></v-list-item>
        </template>
        <v-divider></v-divider>
        <AppTreeview
          :nodes="node.children || []"
          :indent-level="1"
          @expand="(childNode) => $emit('expand', childNode)"
          @node-click="(childNode) => $emit('node-click', childNode)"
          @contextmenu="(data) => $emit('contextmenu', data)"
        />
      </v-list-group>

      <!-- Standard rendering for expandable nodes -->
      <v-list-group v-else-if="node.children">
        <template v-slot:activator="{ props }">
          <v-list-item
            v-bind="props"
            :prepend-icon="node.icon || 'mdi-folder'"
            :title="node.label"
            :style="{ paddingLeft: `${indentLevel * 16}px` }"
            @click.stop="$emit('expand', node)"
            @contextmenu.prevent="$emit('contextmenu', { event: $event, node })"
          ></v-list-item>
        </template>
        <AppTreeview
          :nodes="node.children"
          :indent-level="indentLevel + 1"
          @expand="(childNode) => $emit('expand', childNode)"
          @node-click="(childNode) => $emit('node-click', childNode)"
          @contextmenu="(data) => $emit('contextmenu', data)"
        />
      </v-list-group>

      <!-- Standard rendering for leaf nodes -->
      <v-list-item
        v-else
        :prepend-icon="node.icon || 'mdi-file-document-outline'"
        :title="node.label"
        :style="{ paddingLeft: `${indentLevel * 16}px` }"
        @click="$emit('node-click', node)"
        @contextmenu.prevent="$emit('contextmenu', { event: $event, node })"
      ></v-list-item>
    </template>
  </v-list>
</template>

<script setup lang="ts">
import AppTreeview from './AppTreeview.vue'
import type { ExplorerNode } from '@/stores/explorer'

withDefaults(defineProps<{
  nodes: ExplorerNode[],
  indentLevel?: number
}>(), {
  indentLevel: 0
})

defineEmits<{
  (e: 'node-click', node: ExplorerNode): void
  (e: 'contextmenu', data: { event: MouseEvent, node: ExplorerNode }): void
  (e: 'expand', node: ExplorerNode): void
}>()
</script>

<style scoped>
.connection-banner {
  background-color: #383838;
  font-weight: bold;
}

/* Reduce spacing for prepend-icon */
:deep(.v-list-item__prepend) {
  width: 36px !important;
}
</style>
