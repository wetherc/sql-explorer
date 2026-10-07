<template>
  <div class="notice-host" data-test="notice-stack">
    <!-- One button takes every notice away, because taking four away one at a
         time is work the user should not have to do. It stands above the
         notices, in the place the next one would take. -->
    <div
      v-if="ui.notices.length > 1"
      class="notice notice-bar bg-surface-light"
      data-test="notice-clear-all"
    >
      <span class="text-caption notice-text">{{ ui.notices.length }} notices</span>
      <v-btn
        size="small"
        variant="text"
        text="Dismiss all"
        data-test="notice-clear"
        @click="ui.clear()"
      />
    </div>

    <!-- The newest notice stands at the top of the column, and the oldest
         stays at the bottom edge. Each notice is a live region of its own and
         sits in no other one, so a reader announces it once. An error breaks
         in, because it stops the work of the user. -->
    <div
      v-for="notice in newestFirst"
      :key="notice.id"
      class="notice"
      :class="`bg-${notice.level}`"
      :role="notice.level === 'error' ? 'alert' : 'status'"
      :aria-live="notice.level === 'error' ? 'assertive' : 'polite'"
      data-test="notice"
    >
      <v-icon size="small" aria-hidden="true">{{ notice.icon }}</v-icon>
      <span class="notice-text" :title="notice.message">{{ notice.message }}</span>
      <v-btn
        v-if="notice.detail || notice.level === 'error'"
        size="small"
        variant="text"
        text="Details"
        data-test="notice-details"
        @click="ui.openNotice(notice)"
      />
      <v-btn
        icon="mdi-close"
        size="x-small"
        variant="text"
        aria-label="Dismiss"
        data-test="notice-close"
        @click="ui.dismiss(notice.id)"
      />
    </div>

    <AppDialog
      :model-value="ui.openedNotice !== null"
      size="medium"
      @update:model-value="ui.closeNotice()"
    >
      <v-card v-if="ui.openedNotice">
        <v-card-title class="text-subtitle-1 d-flex align-center ga-2 dialog-title">
          <v-icon :color="ui.openedNotice.level">{{ ui.openedNotice.icon }}</v-icon>
          {{ ui.openedNotice.message }}
        </v-card-title>
        <v-card-text v-if="ui.openedNotice.detail">
          <pre class="app-code-block" data-test="notice-detail-body">{{
            ui.openedNotice.detail
          }}</pre>
        </v-card-text>
        <v-card-actions>
          <v-spacer />
          <v-btn
            prepend-icon="mdi-content-copy"
            text="Copy"
            data-test="notice-copy"
            @click="copyNotice(ui.openedNotice)"
          />
          <v-btn text="Close" data-test="notice-dialog-close" @click="ui.closeNotice()" />
        </v-card-actions>
      </v-card>
    </AppDialog>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from 'vue'
import AppDialog from './AppDialog.vue'
import { useUiStore, type Notice } from '@/stores/ui'

const ui = useUiStore()

const newestFirst = computed(() => [...ui.notices].reverse())

/** The timers of the notices that leave on their own, by notice id. */
const timers = new Map<number, ReturnType<typeof setTimeout>>()

/**
 * Starts a timer for each new notice with a time limit, and stops the timer
 * of each notice that left before its time.
 */
watch(
  () => ui.notices.map((notice) => notice.id),
  () => {
    const current = new Set<number>()
    for (const notice of ui.notices) {
      current.add(notice.id)
      if (notice.timeout >= 0 && !timers.has(notice.id)) {
        const id = notice.id
        timers.set(
          id,
          setTimeout(() => {
            timers.delete(id)
            ui.dismiss(id)
          }, notice.timeout),
        )
      }
    }
    for (const [id, timer] of timers) {
      if (!current.has(id)) {
        clearTimeout(timer)
        timers.delete(id)
      }
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  timers.forEach((timer) => clearTimeout(timer))
  timers.clear()
})

/** Puts the message and the detail of a notice on the clipboard. */
async function copyNotice(notice: Notice): Promise<void> {
  const text = [notice.message, notice.detail].filter(Boolean).join('\n\n')
  const clipboard = globalThis.navigator?.clipboard
  if (!clipboard) {
    ui.warn("Couldn't reach the clipboard, so the text wasn't copied.")
    return
  }
  try {
    await clipboard.writeText(text)
    ui.success('Copied to the clipboard.')
  } catch (error) {
    ui.reportError(error)
  }
}
</script>

<style scoped>
/* The notices stand in one column at the bottom right corner. Each one takes
   the height its text needs, so a long message cannot cover the one above. */
.notice-host {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 2600;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  max-width: calc(100vw - 32px);
  pointer-events: none;
}

.notice {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 480px;
  max-width: 100%;
  min-height: 48px;
  box-sizing: border-box;
  padding: 6px 6px 6px 14px;
  border-radius: 4px;
  box-shadow: 0 3px 8px rgba(0, 0, 0, 0.3);
  font-size: var(--app-text-md);
  pointer-events: auto;
}

/* A long message shows its first three lines, and its title gives the rest. */
.notice-text {
  flex: 1 1 auto;
  min-width: 0;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  line-clamp: 3;
  overflow: hidden;
  overflow-wrap: anywhere;
}

.dialog-title {
  white-space: normal;
  overflow-wrap: anywhere;
}
</style>
