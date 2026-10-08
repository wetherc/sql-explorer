<template>
  <v-alert type="info" density="compact" variant="tonal" data-test="grid-paused">
    Showing the first {{ rows.toLocaleString() }} rows. The query is paused at the row limit, so
    Export all rows can continue it without running it again. Until you export or release it, the
    server keeps the statement open, which can keep locks on the rows it read. It's released
    automatically in {{ pauseLeft(pausedUntil) }}.
    <div v-if="serverLimit" class="app-text-sm mt-1" data-test="grid-paused-server-limit">
      {{ serverLimit }}
    </div>
    <div class="d-flex flex-wrap align-center ga-2 mt-2">
      <v-menu location="bottom start">
        <template #activator="{ props: activator }">
          <v-btn
            v-bind="activator"
            size="small"
            variant="tonal"
            :disabled="exporting"
            data-test="grid-paused-export"
          >
            Export all rows
          </v-btn>
        </template>
        <v-list density="compact">
          <v-list-item
            v-for="entry in pausedExports"
            :key="entry.format"
            :title="entry.title"
            data-test="grid-paused-export-item"
            @click="emit('export-all', entry.format)"
          />
        </v-list>
      </v-menu>
      <v-btn
        size="small"
        variant="text"
        :disabled="exporting"
        data-test="grid-paused-release"
        @click="emit('release')"
      >
        Release
      </v-btn>
      <span :title="pause?.atMost ? 'A query can stay paused for 60 minutes at most.' : undefined">
        <v-btn
          size="small"
          variant="text"
          :disabled="exporting || (pause?.atMost ?? false)"
          data-test="grid-paused-extend"
          @click="emit('extend')"
        >
          +10 min
        </v-btn>
      </span>
      <v-btn
        v-if="pause"
        size="small"
        variant="text"
        data-test="grid-paused-new-tab"
        @click="emit('open-tab', pause.connectionId)"
      >
        Open a new tab on this connection
      </v-btn>
      <v-btn
        v-if="blockedCount > 0"
        size="small"
        variant="tonal"
        color="warning"
        prepend-icon="mdi-database-lock-outline"
        data-test="grid-paused-blocking"
        @click="showBlocking = true"
      >
        Blocking {{ blockedCount }} other {{ blockedCount === 1 ? 'session' : 'sessions' }}
      </v-btn>
    </div>
    <BlockingSessionsDialog
      v-if="pause && showBlocking"
      :open="showBlocking"
      :connection-id="pause.connectionId"
      :blocked-by="pause.serverSession"
      @close="showBlocking = false"
    />
  </v-alert>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import BlockingSessionsDialog from './BlockingSessionsDialog.vue'
import type { ExportAllFormat } from './ResultsGrid.vue'
import type { PauseState } from '@/stores/query'

/**
 * The banner of a result whose read is paused at the row limit. It shows the
 * time left, the sessions that wait for a lock of the read, and the actions
 * on the read.
 */
const props = defineProps<{
  /** The number of rows the grid shows. */
  rows: number
  /** The moment, in milliseconds since the epoch, when the pause ends. */
  pausedUntil: number
  pause?: PauseState
  /** True while an export of all rows runs. */
  exporting: boolean
}>()

const emit = defineEmits<{
  (event: 'export-all', format: ExportAllFormat): void
  (event: 'release'): void
  (event: 'extend'): void
  (event: 'open-tab', connectionId: string): void
}>()

/** The formats that the export of a paused read offers. */
const pausedExports: ReadonlyArray<{ format: ExportAllFormat; title: string }> = [
  { format: 'csv', title: 'CSV' },
  { format: 'json', title: 'JSON' },
  { format: 'xlsx', title: 'Excel' },
]

const showBlocking = ref(false)

/** The number of sessions that wait for a lock of the read. One session
 *  can wait for several locks. */
const blockedCount = computed(
  () => new Set(props.pause?.blocking.map((wait) => wait.waitingSession)).size,
)

/** The longest pause, in seconds. */
const MAX_PAUSE_SECS = 3600

/** A count of seconds in words, as whole minutes when it has no rest. */
function duration(seconds: number): string {
  if (seconds % 60 === 0) {
    const minutes = seconds / 60
    return `${minutes} ${minutes === 1 ? 'minute' : 'minutes'}`
  }
  return `${seconds} ${seconds === 1 ? 'second' : 'seconds'}`
}

/** The note of a server limit on idle transactions that makes the pause
 *  shorter. The app releases the read at nine tenths of that limit. */
const serverLimit = computed(() => {
  const idle = props.pause?.serverIdleSecs
  if (idle === undefined || (idle * 9) / 10 >= MAX_PAUSE_SECS) {
    return ''
  }
  return `The server ends idle transactions after ${duration(idle)}, so the query is released before then.`
})

/** The current time, which updates each second for the time left. */
const now = ref(Date.now())
let pauseClock: ReturnType<typeof setInterval> | undefined

watch(
  () => props.pausedUntil,
  () => {
    clearInterval(pauseClock)
    now.value = Date.now()
    pauseClock = setInterval(() => {
      now.value = Date.now()
    }, 1000)
  },
  { immediate: true },
)

onBeforeUnmount(() => clearInterval(pauseClock))

/** The time left until the end of a pause, as minutes and seconds. */
function pauseLeft(until: number): string {
  const seconds = Math.max(0, Math.ceil((until - now.value) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
</script>
