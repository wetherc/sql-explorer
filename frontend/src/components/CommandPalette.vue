<template>
  <AppDialog :model-value="open" max-width="560" @update:model-value="close">
    <v-card>
      <v-card-text class="pb-2">
        <!-- The field keeps the focus and the list below it holds the choice,
             so the field points a reader at the row that is chosen. -->
        <v-text-field
          v-model="filter"
          autofocus
          density="compact"
          hide-details
          placeholder="Type the name of a command"
          prepend-inner-icon="mdi-magnify"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-listbox"
          :aria-activedescendant="activeId"
          aria-label="Type the name of a command"
          data-test="palette-filter"
          @keydown.down.prevent="move(1)"
          @keydown.up.prevent="move(-1)"
          @keydown.home.prevent="moveTo(0)"
          @keydown.end.prevent="moveTo(matches.length - 1)"
          @keydown.enter.prevent="runSelected"
        />
      </v-card-text>

      <v-list
        id="palette-listbox"
        ref="list"
        density="compact"
        class="palette-list"
        role="listbox"
        aria-label="Commands"
      >
        <v-list-item
          v-for="(command, index) in matches"
          :id="itemId(index)"
          :key="command.id"
          :active="index === selected"
          :disabled="!isEnabled(command)"
          role="option"
          :aria-selected="index === selected"
          data-test="palette-item"
          @click="run(command)"
        >
          <v-list-item-title>{{ command.title }}</v-list-item-title>
          <v-list-item-subtitle>{{ command.group }}</v-list-item-subtitle>
          <template #append>
            <span v-if="command.key" class="palette-key">{{ label(command.key) }}</span>
          </template>
        </v-list-item>

        <v-list-item v-if="matches.length === 0" role="presentation" data-test="palette-empty">
          <v-list-item-title class="text-medium-emphasis">No command matches.</v-list-item-title>
        </v-list-item>
      </v-list>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { computed, nextTick, ref, watch } from 'vue'
import { chordLabel, commandEnabled, filterCommands, type Command } from '@/lib/commands'

const props = defineProps<{ open: boolean; commands: Command[]; apple: boolean }>()
const emit = defineEmits<{ (event: 'update:open', value: boolean): void }>()

const filter = ref('')
const selected = ref(0)
const list = ref<{ $el: HTMLElement } | null>(null)

const matches = computed(() => filterCommands(props.commands, filter.value))

function itemId(index: number): string {
  return `palette-item-${index}`
}

/** The row the field points a reader at, or nothing while no row matches. */
const activeId = computed(() => (matches.value.length > 0 ? itemId(selected.value) : undefined))

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
  if (count > 0) {
    moveTo((selected.value + step + count) % count)
  }
}

/**
 * Chooses one row and brings it into the part of the list that shows. The list
 * scrolls, so a choice made with a key would otherwise walk off the end of it.
 */
function moveTo(index: number): void {
  const count = matches.value.length
  if (count === 0) {
    return
  }
  selected.value = Math.min(count - 1, Math.max(0, index))
  void nextTick(() => {
    const box = list.value?.$el
    box?.querySelector(`#${itemId(selected.value)}`)?.scrollIntoView({ block: 'nearest' })
  })
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
