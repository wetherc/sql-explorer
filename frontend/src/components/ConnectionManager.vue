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
        <v-tooltip location="bottom" text="Refresh">
          <template #activator="{ props: tip }">
            <v-btn
              v-bind="tip"
              icon="mdi-refresh"
              size="small"
              aria-label="Refresh connections"
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
                <v-tooltip location="bottom" :text="healthTip(connection.id)">
                  <template #activator="{ props: tip }">
                    <v-badge
                      v-bind="tip"
                      :model-value="healthColor(connection.id) !== null"
                      :color="healthColor(connection.id) ?? undefined"
                      dot
                      offset-x="2"
                      offset-y="10"
                      data-test="health-dot"
                    >
                      <v-icon
                        :color="connection.color ?? undefined"
                        size="small"
                        aria-hidden="true"
                      >
                        {{ engineIcon(connection.dbType) }}
                      </v-icon>
                    </v-badge>
                  </template>
                </v-tooltip>
              </template>

              <v-list-item-title :title="connection.name" data-test="connection-name">
                {{ connection.name
                }}<span class="d-sr-only">, {{ healthLabel(connection.id) }}</span>
              </v-list-item-title>
              <v-list-item-subtitle :title="subtitle(connection)">
                {{ subtitle(connection) }}
              </v-list-item-subtitle>

              <template #append>
                <v-tooltip
                  location="bottom"
                  :text="connections.isActive(connection.id) ? 'Disconnect' : 'Connect'"
                >
                  <template #activator="{ props: tip }">
                    <v-btn
                      v-bind="tip"
                      :icon="
                        connections.isActive(connection.id)
                          ? 'mdi-lan-disconnect'
                          : 'mdi-lan-connect'
                      "
                      :color="connections.isActive(connection.id) ? 'error' : 'success'"
                      :loading="connections.connecting[connection.id] === true"
                      size="x-small"
                      :aria-label="connections.isActive(connection.id) ? 'Disconnect' : 'Connect'"
                      data-test="toggle-connection"
                      @click.stop="toggle(connection)"
                    />
                  </template>
                </v-tooltip>
                <v-menu>
                  <template #activator="{ props: menu }">
                    <v-tooltip location="bottom" text="More actions">
                      <template #activator="{ props: tip }">
                        <v-btn
                          v-bind="mergeProps(menu, tip)"
                          icon="mdi-dots-vertical"
                          size="x-small"
                          aria-label="More actions"
                          data-test="connection-menu"
                          @click.stop
                        />
                      </template>
                    </v-tooltip>
                  </template>
                  <v-list density="compact">
                    <v-list-item
                      prepend-icon="mdi-pencil-outline"
                      title="Edit…"
                      data-test="edit-connection"
                      @click="startEdit(connection)"
                    />
                    <v-list-item
                      prepend-icon="mdi-content-duplicate"
                      title="Duplicate…"
                      data-test="duplicate-connection"
                      @click="duplicate(connection)"
                    />
                    <v-list-item
                      prepend-icon="mdi-delete-outline"
                      title="Delete…"
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
        hint="Add a server to browse its databases, tables, and columns."
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

    <AppDialog v-if="draft" v-model="editing" size="medium" persistent scrollable>
      <ConnectionForm
        :connection="draft"
        :is-new="isNew"
        :needs-new-token="needsNewToken"
        :is-copy="isCopy"
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
      This removes <strong>{{ pendingDelete.name }}</strong> and its saved password.
      <template v-if="queries.runningOn(pendingDelete.id) > 0">
        <br /><br /><span data-test="delete-running">{{ runningMessage(pendingDelete.id) }}</span>
      </template>
    </ConfirmDialog>

    <ConfirmDialog
      v-if="pendingDisconnect"
      :open="pendingDisconnect !== null"
      title="Disconnect?"
      :message="runningMessage(pendingDisconnect.id)"
      confirm-text="Disconnect"
      danger
      @confirm="confirmDisconnect(pendingDisconnect)"
      @cancel="pendingDisconnect = null"
    />
  </div>
</template>

<script setup lang="ts">
import AppDialog from './AppDialog.vue'
import { mergeProps, ref, watch } from 'vue'
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
/** True while the form edits a copy, whose secrets stay with the original. */
const isCopy = ref(false)
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

/** The last failure to connect to one server, if its last attempt failed. */
function lastError(id: string): string | undefined {
  return connections.lastError[id]
}

/** The colour of the dot beside a connection, or null for no dot. */
function healthColor(id: string): string | null {
  switch (connections.health[id]) {
    case ConnectionHealth.Connected:
      return 'success'
    case ConnectionHealth.Reconnecting:
      return 'warning'
    default:
      return lastError(id) ? 'error' : null
  }
}

function healthLabel(id: string): string {
  switch (connections.health[id]) {
    case ConnectionHealth.Connected:
      return 'Connected'
    case ConnectionHealth.Reconnecting:
      return 'Reconnecting'
    default:
      return lastError(id) ? 'Connection failed' : 'Not connected'
  }
}

/** The tooltip of the dot, which adds the reason of a failure. */
function healthTip(id: string): string {
  const failure = connections.health[id] === ConnectionHealth.Connected ? undefined : lastError(id)
  return failure ? `${healthLabel(id)}: ${failure}` : healthLabel(id)
}

function startNew(): void {
  draft.value = newConnection()
  isNew.value = true
  isCopy.value = false
  needsNewToken.value = false
  editing.value = true
}

function startEdit(connection: SavedConnection): void {
  draft.value = { ...connection, options: { ...connection.options }, password: '' }
  isNew.value = false
  isCopy.value = false
  needsNewToken.value = false
  editing.value = true
}

function duplicate(connection: SavedConnection): void {
  draft.value = connections.duplicate(connection)
  isNew.value = true
  isCopy.value = true
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
  await connections.remove(connection.id)
  // A delete that failed leaves the record in the list, and its tree stays.
  if (!connections.byId(connection.id)) {
    explorer.removeRoot(connection.id)
  }
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
    // A connection that the backend dropped keeps its root, with the objects
    // and the schema of the last session. The record can point at another
    // server since then, so the old root goes and a new one is read.
    explorer.removeRoot(connection.id)
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
