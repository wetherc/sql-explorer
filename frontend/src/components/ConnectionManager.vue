<template>
  <div class="connection-manager">
    <PanelHeader>
      <template #lead>
        <v-btn
          color="primary"
          variant="flat"
          size="small"
          prepend-icon="mdi-plus"
          text="New"
          data-test="new-connection"
          @click="startNew"
        />
      </template>
      <template #actions>
        <v-tooltip location="bottom" text="Read the connections again">
          <template #activator="{ props: tip }">
            <v-btn
              v-bind="tip"
              icon="mdi-refresh"
              size="small"
              aria-label="Read the connections again"
              data-test="refresh-connections"
              @click="connections.load()"
            />
          </template>
        </v-tooltip>
      </template>
    </PanelHeader>

    <v-progress-linear v-if="connections.loading" indeterminate height="2" />

    <div class="list-body">
      <template v-if="connections.saved.length > 0">
        <div v-for="group in connections.groups" :key="group" class="group">
          <div class="group-title px-3 py-1">{{ group }}</div>
          <v-list density="compact" class="pa-0">
            <v-list-item
              v-for="connection in inGroup(group)"
              :key="connection.id"
              :active="connections.selectedId === connection.id"
              class="connection-item"
              data-test="connection-item"
              @click="selectConnection(connection)"
            >
              <template #prepend>
                <v-badge
                  :color="healthColor(connection.id)"
                  dot
                  offset-x="2"
                  offset-y="10"
                  :aria-label="healthLabel(connection.id)"
                >
                  <v-icon :color="connection.color ?? undefined" size="small">
                    {{ engineIcon(connection.dbType) }}
                  </v-icon>
                </v-badge>
              </template>

              <v-list-item-title>{{ connection.name }}</v-list-item-title>
              <v-list-item-subtitle>{{ subtitle(connection) }}</v-list-item-subtitle>

              <template #append>
                <v-btn
                  :icon="
                    connections.isActive(connection.id) ? 'mdi-lan-disconnect' : 'mdi-lan-connect'
                  "
                  :color="connections.isActive(connection.id) ? 'error' : 'success'"
                  :loading="connections.connecting[connection.id] === true"
                  size="x-small"
                  :aria-label="connections.isActive(connection.id) ? 'Close' : 'Open'"
                  data-test="toggle-connection"
                  @click.stop="toggle(connection)"
                />
                <v-menu>
                  <template #activator="{ props: menu }">
                    <v-btn
                      v-bind="menu"
                      icon="mdi-dots-vertical"
                      size="x-small"
                      aria-label="More actions"
                      data-test="connection-menu"
                      @click.stop
                    />
                  </template>
                  <v-list density="compact">
                    <v-list-item
                      prepend-icon="mdi-pencil"
                      title="Edit"
                      data-test="edit-connection"
                      @click="startEdit(connection)"
                    />
                    <v-list-item
                      prepend-icon="mdi-content-duplicate"
                      title="Duplicate"
                      data-test="duplicate-connection"
                      @click="duplicate(connection)"
                    />
                    <v-list-item
                      prepend-icon="mdi-delete"
                      title="Delete"
                      base-color="error"
                      data-test="delete-connection"
                      @click="askDelete(connection)"
                    />
                  </v-list>
                </v-menu>
              </template>
            </v-list-item>
          </v-list>
        </div>
      </template>

      <EmptyState
        v-else
        icon="mdi-lan-pending"
        title="No connections yet"
        hint="Add a server to see its databases, tables and columns."
      >
        <v-btn
          color="primary"
          variant="flat"
          size="small"
          prepend-icon="mdi-plus"
          text="New connection"
          data-test="empty-new-connection"
          @click="startNew"
        />
      </EmptyState>
    </div>

    <AppDialog v-if="draft" v-model="editing" max-width="620" persistent scrollable>
      <ConnectionForm
        :connection="draft"
        :is-new="isNew"
        :needs-new-token="needsNewToken"
        @close="editing = false"
        @saved="onSaved"
      />
    </AppDialog>

    <ConfirmDialog
      v-if="pendingDelete"
      :open="deleting"
      title="Delete this connection?"
      confirm-text="Delete"
      danger
      @confirm="confirmDelete(pendingDelete)"
      @cancel="deleting = false"
    >
      The record for <strong>{{ pendingDelete.name }}</strong> and its password are removed.
    </ConfirmDialog>

    <ConfirmDialog
      v-if="pendingDisconnect"
      :open="pendingDisconnect !== null"
      title="Close this connection?"
      :message="runningMessage(pendingDisconnect.id)"
      confirm-text="Close it"
      danger
      @confirm="confirmDisconnect(pendingDisconnect)"
      @cancel="pendingDisconnect = null"
    />
  </div>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { ref, watch } from 'vue'
import ConnectionForm from './ConnectionForm.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import EmptyState from './EmptyState.vue'
import PanelHeader from './PanelHeader.vue'
import { stoppedStatementsMessage } from '@/lib/format'
import { connectionSubtitle, newConnection, useConnectionsStore } from '@/stores/connections'
import { useExplorerStore } from '@/stores/explorer'
import { useQueryStore } from '@/stores/query'
import { ConnectionHealth, DbType, type SavedConnection } from '@/types/api'

const connections = useConnectionsStore()
const explorer = useExplorerStore()
const queries = useQueryStore()

const emit = defineEmits<{ (event: 'connected', id: string): void }>()

const editing = ref(false)
const isNew = ref(true)
const draft = ref<SavedConnection | null>(null)
const deleting = ref(false)
/** True while the form asks for an access token that is fresh. */
const needsNewToken = ref(false)
const pendingDelete = ref<SavedConnection | null>(null)
/** The connection that waits on an answer, while statements run on it. */
const pendingDisconnect = ref<SavedConnection | null>(null)

function inGroup(group: string): SavedConnection[] {
  return connections.saved.filter(
    (connection) => (connection.group?.trim() || 'Connections') === group,
  )
}

function subtitle(connection: SavedConnection): string {
  return connectionSubtitle(connection)
}

function engineIcon(dbType: DbType): string {
  switch (dbType) {
    case DbType.Mssql:
      return 'mdi-microsoft'
    case DbType.Athena:
      return 'mdi-aws'
    case DbType.Postgres:
      return 'mdi-elephant'
    case DbType.Mysql:
      return 'mdi-dolphin'
    default:
      return 'mdi-file-cabinet'
  }
}

function healthColor(id: string): string {
  switch (connections.health[id]) {
    case ConnectionHealth.Connected:
      return 'success'
    case ConnectionHealth.Reconnecting:
      return 'warning'
    default:
      return 'transparent'
  }
}

function healthLabel(id: string): string {
  return connections.health[id] ?? 'not connected'
}

function startNew(): void {
  draft.value = newConnection()
  isNew.value = true
  needsNewToken.value = false
  editing.value = true
}

function startEdit(connection: SavedConnection): void {
  draft.value = { ...connection, options: { ...connection.options }, password: '' }
  isNew.value = false
  needsNewToken.value = false
  editing.value = true
}

function duplicate(connection: SavedConnection): void {
  draft.value = connections.duplicate(connection)
  isNew.value = true
  needsNewToken.value = false
  editing.value = true
}

// A login that failed while the connection held a pasted token opens the
// form, because the stored token cannot be made fresh again.
watch(
  () => connections.expiredTokenId,
  (id) => {
    if (!id) {
      return
    }
    const connection = connections.byId(id)
    connections.clearExpiredToken()
    if (!connection) {
      return
    }
    startEdit(connection)
    needsNewToken.value = true
  },
)

function askDelete(connection: SavedConnection): void {
  pendingDelete.value = connection
  deleting.value = true
}

async function confirmDelete(connection: SavedConnection): Promise<void> {
  deleting.value = false
  pendingDelete.value = null
  explorer.removeRoot(connection.id)
  await connections.remove(connection.id)
}

async function toggle(connection: SavedConnection): Promise<void> {
  if (connections.isActive(connection.id)) {
    // A close stops every statement that runs on the connection, so it asks
    // first when one does. A connection with nothing running closes at once,
    // because it takes nothing away.
    if (queries.runningOn(connection.id) > 0) {
      pendingDisconnect.value = connection
      return
    }
    await closeConnection(connection.id)
    return
  }
  const opened = await connections.connect(connection)
  if (opened) {
    const root = explorer.addRoot(connection.id)
    await explorer.expand(root)
    emit('connected', connection.id)
  }
}

/** Says how many statements the close of one connection would stop. */
function runningMessage(id: string): string {
  return stoppedStatementsMessage(queries.runningOn(id))
}

async function closeConnection(id: string): Promise<void> {
  await connections.disconnect(id)
  explorer.removeRoot(id)
}

async function confirmDisconnect(connection: SavedConnection): Promise<void> {
  pendingDisconnect.value = null
  await closeConnection(connection.id)
}

function selectConnection(connection: SavedConnection): void {
  if (connections.isActive(connection.id)) {
    connections.select(connection.id)
  }
}

function onSaved(): void {
  editing.value = false
}
</script>

<style scoped>
.connection-manager {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.list-body {
  flex: 1 1 auto;
  overflow: auto;
  min-height: 0;
}

.group-title {
  font-size: var(--app-text-xs);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: rgb(var(--v-theme-on-surface-variant));
}

.connection-item {
  padding-inline-start: 10px !important;
}
</style>
