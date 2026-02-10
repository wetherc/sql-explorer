<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import { useToast } from 'primevue/usetoast'
import { buildConnectionString } from '@/utils/connectionStringBuilder'
import type { SavedConnection } from '@/types/savedConnection'
import { AuthType, DbType } from '@/types/savedConnection'

import SavedConnections from '@/components/SavedConnections.vue'
import SaveConnectionDialog from '@/components/SaveConnectionDialog.vue'

import Button from 'primevue/button'
import Card from 'primevue/card'
import Checkbox from 'primevue/checkbox'
import Dropdown from 'primevue/dropdown'
import FloatLabel from 'primevue/floatlabel'
import InputNumber from 'primevue/inputnumber'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Password from 'primevue/password'

const connectionStore = useConnectionStore()
const savedConnectionsStore = useSavedConnectionsStore()
const toast = useToast()

const dbType = ref<DbType>(DbType.Mssql)
const server = ref('localhost')
const port = ref<number | undefined>(1433)
const database = ref('master')
const applicationName = ref('sql-explorer')
const connectTimeout = ref(15)
const username = ref('sa')
const password = ref('')
const authType = ref<AuthType>(AuthType.Sql)
const trustServerCertificate = ref(true)

const showSaveDialog = ref(false)

const isSqlAuth = computed(() => authType.value === AuthType.Sql)
const isNamedInstance = computed(() => dbType.value === DbType.Mssql && server.value.includes('\\'))

const dbTypeOptions = ref([
  { label: 'Microsoft SQL Server', value: DbType.Mssql },
  { label: 'MySQL', value: DbType.Mysql },
  { label: 'PostgreSQL', value: DbType.Postgres },
])

const authTypeOptions = computed(() => {
  const options = [{ label: 'SQL Authentication', value: AuthType.Sql }]
  if (dbType.value === DbType.Mssql) {
    options.push({ label: 'Microsoft Entra / Integrated', value: AuthType.Integrated })
  }
  return options
})


watch(isNamedInstance, (isNamed) => {
  if (isNamed) {
    port.value = undefined
  } else if (dbType.value === DbType.Mssql) {
    port.value = 1433
  }
})

watch(dbType, (newDbType) => {
  if (newDbType === DbType.Mysql) {
    port.value = 3306
    authType.value = AuthType.Sql // MySQL doesn't have integrated auth
  } else if (newDbType === DbType.Postgres) {
    port.value = 5432
    authType.value = AuthType.Sql // Postgres also uses standard user/pass
  } else {
    port.value = 1433
  }
})

async function handleConnect() {
  let finalPassword = password.value
  // If we are using SQL auth and the password field is empty,
  // it could be a saved connection. Try to fetch the password.
  if (isSqlAuth.value && !password.value) {
    const matchingSavedConn = savedConnectionsStore.connections.find(
      (c) =>
        c.db_type === dbType.value &&
        c.server === server.value &&
        c.database === database.value &&
        c.user === username.value
    )
    if (matchingSavedConn) {
      const savedPassword = await savedConnectionsStore.getPassword(matchingSavedConn.name)
      if (savedPassword) {
        finalPassword = savedPassword
      }
    }
  }

  try {
    // TODO: Pass the selected TLS/SSL mode (from the new UI element) to buildConnectionString
    const connectionString = buildConnectionString({
      dbType: dbType.value,
      server: server.value,
      port: port.value,
      database: database.value,
      applicationName: applicationName.value,
      connectTimeout: connectTimeout.value,
      authType: authType.value,
      username: isSqlAuth.value ? username.value : undefined,
      password: isSqlAuth.value ? finalPassword : undefined,
      trustServerCertificate: trustServerCertificate.value
    })
    await connectionStore.connect(connectionString, dbType.value)
    if (connectionStore.isConnected) {
      toast.add({ severity: 'success', summary: 'Connected', detail: `Successfully connected to ${server.value}`, life: 3000 });
    }
  } catch (error: unknown) {
    connectionStore.errorMessage =
      error instanceof Error ? error.message : 'An unknown error occurred.'
  }
}

function handleSelectSavedConnection(connection: SavedConnection) {
  dbType.value = connection.db_type
  server.value = connection.server
  database.value = connection.database
  authType.value = connection.auth_type
  if (connection.user) {
    username.value = connection.user
  } else {
    username.value = ''
  }
  // Clear the password field. It will be fetched on connect if needed.
  password.value = ''
}

function openSaveDialog() {
  showSaveDialog.value = true
}

async function handleSaveConnection(name: string, passwordToSave?: string) {
  const connectionToSave: Omit<SavedConnection, 'name'> = {
    db_type: dbType.value,
    server: server.value,
    database: database.value,
    auth_type: authType.value,
    user: isSqlAuth.value ? username.value : undefined
  }
  try {
    await savedConnectionsStore.saveConnection({ name, ...connectionToSave }, passwordToSave)
    toast.add({ severity: 'success', summary: 'Saved', detail: `Connection '${name}' saved.`, life: 3000 });
  } catch (error) {
    toast.add({ severity: 'error', summary: 'Error', detail: `Failed to save connection: ${error}`, life: 5000 });
  }
}
</script>

<template>
  <div class="connection-view-wrapper">
    <div class="connection-view">
      <Card>
        <template #title>
          <div class="text-center">Connect to Database</div>
        </template>
        <template #content>
          <form @submit.prevent="handleConnect" class="p-fluid">
            <div class="field">
              <FloatLabel>
                <Dropdown id="db-type" v-model="dbType" :options="dbTypeOptions" option-label="label" option-value="value" :disabled="connectionStore.isConnecting" />
                <label for="db-type">Database Type</label>
              </FloatLabel>
            </div>

            <div class="field">
              <FloatLabel>
                <InputText id="server" v-model="server" required :disabled="connectionStore.isConnecting" />
                <label for="server">Server or Server\Instance</label>
              </FloatLabel>
            </div>

            <div class="field">
              <FloatLabel>
                <InputNumber id="port" v-model="port" :required="!isNamedInstance" :disabled="connectionStore.isConnecting || isNamedInstance" />
                <label for="port">Port</label>
              </FloatLabel>
            </div>

            <div class="field">
              <FloatLabel>
                <InputText id="database" v-model="database" :disabled="connectionStore.isConnecting" />
                <label for="database">Database (optional)</label>
              </FloatLabel>
            </div>

            <div class="field">
              <FloatLabel>
                <InputText id="appName" v-model="applicationName" :disabled="connectionStore.isConnecting" />
                <label for="appName">Application Name</label>
              </FloatLabel>
            </div>

             <div class="field">
              <FloatLabel>
                <InputNumber id="timeout" v-model="connectTimeout" required :disabled="connectionStore.isConnecting" />
                <label for="timeout">Connect Timeout (seconds)</label>
              </FloatLabel>
            </div>

            <div v-if="dbType === DbType.Mssql" class="field-checkbox">
              <Checkbox v-model="trustServerCertificate" input-id="trustServerCertificate" :binary="true" :disabled="connectionStore.isConnecting" />
              <label for="trustServerCertificate" class="ml-2"> Trust server certificate </label>
            </div>
            
            <!-- TODO: Add UI element (e.g., Dropdown) for TLS/SSL mode selection (Disabled, Preferred, Required, Verify CA, Verify Full) -->
            <!-- This will allow users to configure the new optional TLS features in the backend. -->
            
            <div class="field">
              <FloatLabel>
                <Dropdown id="auth-type" v-model="authType" :options="authTypeOptions" option-label="label" option-value="value" :disabled="connectionStore.isConnecting" />
                <label for="auth-type">Authentication</label>
              </FloatLabel>
            </div>

            <template v-if="isSqlAuth">
              <div class="field">
                <FloatLabel>
                  <InputText id="username" v-model="username" required :disabled="connectionStore.isConnecting" />
                  <label for="username">Username</label>
                </FloatLabel>
              </div>

              <div class="field">
                <FloatLabel>
                  <Password id="password" v-model="password" :feedback="false" toggle-mask :disabled="connectionStore.isConnecting" />
                  <label for="password">Password</label>
                </FloatLabel>
              </div>
            </template>

            <div class="form-actions">
              <Button type="submit" label="Connect" :loading="connectionStore.isConnecting" icon="pi pi-check" class="p-button-success" />
              <Button type="button" label="Save" @click="openSaveDialog" :disabled="connectionStore.isConnecting" icon="pi pi-save" class="p-button-secondary" />
            </div>

            <Message v-if="connectionStore.errorMessage" severity="error" :closable="false">{{ connectionStore.errorMessage }}</Message>
          </form>
        </template>
      </Card>
    </div>
    <div class="saved-connections-panel">
      <SavedConnections @select="handleSelectSavedConnection" />
    </div>

    <SaveConnectionDialog
      v-model:visible="showSaveDialog"
      :connection="{
        db_type: dbType,
        server,
        database,
        auth_type: authType,
        user: isSqlAuth ? username : undefined
      }"
      :password="isSqlAuth ? password : undefined"
      @save="handleSaveConnection"
    />
  </div>
</template>

<style scoped>
.connection-view-wrapper {
  display: grid;
  grid-template-columns: 2fr 1fr;
  height: 100vh;
  gap: 1rem;
  padding: 1rem;
}
.connection-view {
  display: flex;
  justify-content: center;
  align-items: flex-start;
  overflow-y: auto;
}

.field, .field-checkbox {
  margin-bottom: 1.5rem;
}

.form-actions {
  display: flex;
  gap: 1rem;
  margin-top: 1.5rem;
}

.saved-connections-panel {
  border-left: 1px solid var(--p-surface-border);
  padding-left: 1rem;
  overflow-y: auto;
}
</style>
