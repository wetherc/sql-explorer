<template>
  <AppDialog :model-value="open" size="large" scrollable @update:model-value="emit('close')">
    <v-card data-test="blocking-dialog">
      <v-card-title>{{ title }}</v-card-title>

      <v-card-text>
        <v-progress-linear v-if="loading" indeterminate class="mb-3" data-test="blocking-loading" />

        <!-- The dialog shows its own failure, so the user can read the
             report again with Refresh. -->
        <v-alert
          v-if="failure"
          type="error"
          variant="tonal"
          density="compact"
          data-test="blocking-error"
        >
          <div class="font-weight-medium">{{ failure.message }}</div>
          <pre v-if="failure.detail" class="app-code-block mt-2">{{ failure.detail }}</pre>
        </v-alert>

        <template v-if="report">
          <v-alert
            v-for="note of report.notes"
            :key="note"
            type="info"
            variant="tonal"
            density="compact"
            class="mb-3"
            data-test="blocking-note"
          >
            {{ note }}
          </v-alert>

          <div class="app-heading mb-1">Waiting for a lock</div>
          <p
            v-if="report.sessions.length === 0"
            class="text-medium-emphasis mb-4"
            data-test="no-waits"
          >
            No session is waiting for a lock right now.
          </p>
          <div
            v-for="(wait, index) of report.sessions"
            :key="`wait ${index}`"
            class="blocking-entry"
            data-test="blocking-wait"
          >
            <div class="font-weight-medium">
              Session {{ wait.waitingSession }} is waiting for session {{ wait.blockingSession }}
            </div>
            <dl class="blocking-facts">
              <template v-for="fact of waitFacts(wait)" :key="fact.name">
                <dt>{{ fact.name }}</dt>
                <dd>{{ fact.value }}</dd>
              </template>
            </dl>
            <div
              v-for="statement of waitStatements(wait)"
              :key="statement.label"
              class="blocking-statement"
            >
              <div class="statement-header">
                <span class="text-medium-emphasis">{{ statement.label }}</span>
                <v-btn
                  icon="mdi-content-copy"
                  size="x-small"
                  variant="text"
                  :aria-label="`Copy ${statement.label.toLowerCase()}`"
                  data-test="blocking-copy"
                  @click="copy(statement.text)"
                />
              </div>
              <pre class="app-code-block">{{ statement.text }}</pre>
            </div>
          </div>

          <div class="app-heading mt-2 mb-1">Open transactions</div>
          <p
            v-if="report.openTransactions.length === 0"
            class="text-medium-emphasis"
            data-test="no-transactions"
          >
            No other session has an open transaction.
          </p>
          <div
            v-for="transaction of report.openTransactions"
            :key="transaction.session"
            class="blocking-entry"
            data-test="blocking-transaction"
          >
            <div class="font-weight-medium">Session {{ transaction.session }}</div>
            <dl class="blocking-facts">
              <template v-for="fact of transactionFacts(transaction)" :key="fact.name">
                <dt>{{ fact.name }}</dt>
                <dd>{{ fact.value }}</dd>
              </template>
            </dl>
            <div v-if="transaction.statement" class="blocking-statement">
              <div class="statement-header">
                <span class="text-medium-emphasis">Last statement</span>
                <v-btn
                  icon="mdi-content-copy"
                  size="x-small"
                  variant="text"
                  aria-label="Copy last statement"
                  data-test="blocking-copy"
                  @click="copy(transaction.statement)"
                />
              </div>
              <pre class="app-code-block">{{ transaction.statement }}</pre>
            </div>
          </div>
        </template>
      </v-card-text>

      <v-card-actions>
        <v-spacer />
        <v-btn
          prepend-icon="mdi-refresh"
          text="Refresh"
          :disabled="loading"
          data-test="blocking-refresh"
          @click="read"
        />
        <v-btn text="Close" data-test="blocking-close" @click="emit('close')" />
      </v-card-actions>
    </v-card>
  </AppDialog>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { computed, ref, watch } from 'vue'
import { api } from '@/lib/api'
import { toErrorPayload } from '@/lib/errors'
import { formatDuration } from '@/lib/format'
import { useUiStore } from '@/stores/ui'
import type { BlockingReport, BlockingSession, ErrorPayload, OpenTransaction } from '@/types/api'

/**
 * The report of the sessions that wait for locks and the sessions that keep
 * them. With `blockedBy`, the report shows the rows of that one session.
 */
const props = defineProps<{ open: boolean; connectionId: string; blockedBy?: number | null }>()
const emit = defineEmits<{ (event: 'close'): void }>()

const ui = useUiStore()
const report = ref<BlockingReport | null>(null)
const loading = ref(false)
const failure = ref<ErrorPayload | null>(null)

const title = computed(() =>
  (props.blockedBy ?? null) === null ? 'Blocking sessions' : `Locks of session ${props.blockedBy}`,
)

interface Fact {
  name: string
  value: string
}

/** Keeps the facts that the server gave, in the order of the list. */
function facts(entries: [string, string | null][]): Fact[] {
  return entries
    .filter((entry): entry is [string, string] => entry[1] !== null)
    .map(([name, value]) => ({ name, value }))
}

function waitFacts(wait: BlockingSession): Fact[] {
  return facts([
    ['Object', wait.object],
    ['Lock', wait.lockMode],
    ['Waiting for', wait.waitMs === null ? null : formatDuration(wait.waitMs)],
    ['Blocker login', wait.blockingLogin],
    ['Blocker host', wait.blockingHost],
    ['Blocker program', wait.blockingProgram],
    ['Blocker status', wait.blockingStatus],
  ])
}

function transactionFacts(transaction: OpenTransaction): Fact[] {
  return facts([
    [
      'Open for',
      transaction.openSecs === null ? null : formatDuration(transaction.openSecs * 1000),
    ],
    ['Login', transaction.login],
    ['Host', transaction.host],
    ['Program', transaction.program],
    ['Status', transaction.status],
  ])
}

/** The statements of one wait that the server gave, the blocker first. */
function waitStatements(wait: BlockingSession): { label: string; text: string }[] {
  const out: { label: string; text: string }[] = []
  if (wait.blockingStatement) {
    out.push({ label: 'Blocking statement', text: wait.blockingStatement })
  }
  if (wait.waitingStatement) {
    out.push({ label: 'Waiting statement', text: wait.waitingStatement })
  }
  return out
}

/**
 * Counts the reads that the dialog started. A read writes into the dialog
 * only while its count is the last one, so a slow answer for an earlier
 * connection goes away.
 */
let lastRead = 0

async function read(): Promise<void> {
  lastRead += 1
  const ticket = lastRead
  loading.value = true
  failure.value = null
  try {
    const answer = await api.blockingSessions(props.connectionId, props.blockedBy ?? null)
    if (ticket === lastRead) {
      report.value = answer
    }
  } catch (error) {
    if (ticket === lastRead) {
      report.value = null
      failure.value = toErrorPayload(error)
    }
  } finally {
    if (ticket === lastRead) {
      loading.value = false
    }
  }
}

async function copy(text: string): Promise<void> {
  const clipboard = globalThis.navigator?.clipboard
  if (!clipboard) {
    ui.warn("Couldn't reach the clipboard, so the statement wasn't copied.")
    return
  }
  try {
    await clipboard.writeText(text)
    ui.success('Statement copied.')
  } catch (error) {
    ui.reportError(error)
  }
}

watch(
  () => [props.open, props.connectionId, props.blockedBy],
  () => {
    if (props.open) {
      void read()
    }
  },
  { immediate: true },
)
</script>

<style scoped>
.blocking-entry {
  padding: 8px 0 12px;
  border-bottom: var(--app-divider-soft);
  margin-bottom: 8px;
}

.blocking-facts {
  display: grid;
  grid-template-columns: max-content 1fr;
  column-gap: 16px;
  margin: 4px 0;
}

.blocking-facts dt {
  color: rgba(var(--v-theme-on-surface), var(--v-medium-emphasis-opacity));
}

.blocking-facts dd {
  margin: 0;
  word-break: break-word;
}

.blocking-statement {
  margin-top: 6px;
}

.statement-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.blocking-statement .app-code-block {
  max-height: 160px;
}
</style>
