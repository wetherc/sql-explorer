<template>
  <div v-if="hiddenCount > 0" class="text-medium-emphasis mb-1" data-test="messages-hidden">
    {{ hiddenText }}
  </div>

  <div
    v-for="(message, index) in shown"
    :key="hiddenCount + index"
    class="message-line"
    :class="`message-${message.level}`"
    data-test="query-message"
  >
    <v-icon v-if="message.level !== 'info'" size="x-small" class="mr-1">
      {{ message.level === 'error' ? 'mdi-alert-circle' : 'mdi-alert' }}
    </v-icon>
    {{ message.text }}
    <span v-if="message.detail" class="message-detail">{{ message.detail }}</span>
  </div>

  <div
    v-if="!hasError && messages.length === 0"
    class="text-medium-emphasis"
    data-test="no-messages"
  >
    Messages from the server appear here when you run a statement.
  </div>
</template>

<script setup lang="ts">
/**
 * The messages of the server for one tab.
 *
 * A loop that prints a line for each row can send tens of thousands of
 * messages. The list therefore draws the last `MAX_SHOWN_MESSAGES` of them
 * and counts the others in one line above them. That count includes the
 * messages that the store dropped.
 *
 * The component reads the length of the list through the store and the
 * messages from the plain array. A new message then changes this component
 * alone, and it wraps no message of the list in a proxy. The key of a line
 * is its place in the whole list, so a new message adds one line and takes
 * away the first one, and the lines between them stay. When the store drops
 * messages, the dropped count rises by the length that the list loses, so
 * the key of each line stays the same.
 */
import { computed, toRaw } from 'vue'
import type { Message } from '@/types/api'

/** The most messages the list draws. */
const MAX_SHOWN_MESSAGES = 500

const props = defineProps<{
  /** The last messages of the run, as the store keeps them. */
  messages: Message[]
  /** The count of the first messages of the run that the store dropped. */
  dropped: number
  /** True when the tab shows a failure, which takes the place of the empty note. */
  hasError: boolean
}>()

const hiddenCount = computed(
  () => props.dropped + Math.max(0, props.messages.length - MAX_SHOWN_MESSAGES),
)

// The length comes through the store, so each new message reaches the list.
const shown = computed(() => {
  const count = props.messages.length
  return toRaw(props.messages).slice(Math.max(0, count - MAX_SHOWN_MESSAGES))
})

const hiddenText = computed(() =>
  hiddenCount.value === 1
    ? '1 earlier message not shown'
    : `${hiddenCount.value.toLocaleString()} earlier messages not shown`,
)
</script>

<style scoped>
.message-warning {
  color: rgb(var(--v-theme-warning));
}

.message-error {
  color: rgb(var(--v-theme-error));
}

/* The detail of one message stays in the colour of its level, so a smaller
   size is what separates it from the text of the message. */
.message-detail {
  margin-left: 0.5rem;
  font-size: var(--app-text-sm);
}

.message-line {
  padding: 2px 0;
  font-family: var(--app-font-mono);
}
</style>
