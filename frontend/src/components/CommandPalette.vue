<template>
  <v-dialog :model-value="open" max-width="560" @update:model-value="close">
    <v-card>
      <v-card-text class="pb-2">
        <v-text-field
          v-model="filter"
          autofocus
          density="compact"
          hide-details
          placeholder="Type the name of a command"
          prepend-inner-icon="mdi-magnify"
          data-test="palette-filter"
          @keydown.down.prevent="move(1)"
          @keydown.up.prevent="move(-1)"
          @keydown.enter.prevent="runSelected"
        />
      </v-card-text>

      <v-list density="compact" class="palette-list">
        <v-list-item
          v-for="(command, index) in matches"
          :key="command.id"
          :active="index === selected"
          :disabled="!isEnabled(command)"
          data-test="palette-item"
          @click="run(command)"
        >
          <v-list-item-title>{{ command.title }}</v-list-item-title>
          <v-list-item-subtitle>{{ command.group }}</v-list-item-subtitle>
          <template #append>
            <span v-if="command.key" class="palette-key">{{ label(command.key) }}</span>
          </template>
        </v-list-item>

        <v-list-item v-if="matches.length === 0" data-test="palette-empty">
          <v-list-item-title class="text-medium-emphasis">No command matches.</v-list-item-title>
        </v-list-item>
      </v-list>
    </v-card>
  </v-dialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { chordLabel, commandEnabled, filterCommands, type Command } from '@/lib/commands'

const props = defineProps<{ open: boolean; commands: Command[]; apple: boolean }>()
const emit = defineEmits<{ (event: 'update:open', value: boolean): void }>()

const filter = ref('')
const selected = ref(0)

const matches = computed(() => filterCommands(props.commands, filter.value))

function isEnabled(command: Command): boolean {
  return commandEnabled(command)
}

function label(key: string): string {
  return chordLabel(key, props.apple)
}

function close(): void {
  emit('update:open', false)
}

function move(step: number): void {
  const count = matches.value.length
  if (count === 0) {
    return
  }
  selected.value = (selected.value + step + count) % count
}

function run(command: Command): void {
  if (!isEnabled(command)) {
    return
  }
  close()
  command.run()
}

function runSelected(): void {
  const command = matches.value[selected.value]
  if (command) {
    run(command)
  }
}

// Every opening starts with an empty filter and the first command selected.
watch(
  () => props.open,
  (open) => {
    if (open) {
      filter.value = ''
      selected.value = 0
    }
  },
)

watch(filter, () => {
  selected.value = 0
})
</script>

<style scoped>
.palette-list {
  max-height: 320px;
  overflow: auto;
}

.palette-key {
  font-size: var(--app-text-sm);
  color: rgb(var(--v-theme-on-surface-variant));
  white-space: nowrap;
}
</style>
