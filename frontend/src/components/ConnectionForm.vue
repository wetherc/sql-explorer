<template>
  <v-card>
    <v-card-title class="text-subtitle-1">
      {{ isNew ? 'New connection' : `Edit ${draft.name || 'connection'}` }}
    </v-card-title>

    <v-card-text class="form-body">
      <v-select
        v-model="draft.dbType"
        :items="engineItems"
        item-title="title"
        item-value="value"
        label="Engine"
        data-test="engine-select"
        @update:model-value="onEngineChange"
      />

      <v-text-field v-model="draft.name" label="Name" data-test="name-field" />

      <template v-if="engine?.usesHost">
        <div class="d-flex ga-2">
          <v-text-field
            v-model="draft.host"
            label="Host"
            class="flex-grow-1"
            data-test="host-field"
          />
          <v-text-field
            v-model.number="draft.port"
            label="Port"
            type="number"
            style="max-width: 130px"
            data-test="port-field"
          />
        </div>
      </template>

      <v-text-field
        v-if="engine?.usesFile"
        v-model="draft.options.filePath"
        label="Database file"
        data-test="file-field"
      >
        <template #append-inner>
          <v-btn
            icon="mdi-folder-open-outline"
            size="x-small"
            aria-label="Choose a file"
            data-test="choose-file"
            @click="chooseFile"
          />
        </template>
      </v-text-field>

      <v-select
        v-if="engine?.supportsIntegratedSecurity"
        v-model="draft.options.mssqlAuth"
        :items="authItems"
        item-title="title"
        item-value="value"
        label="Authentication"
        :hint="authHint"
        persistent-hint
        data-test="auth-select"
      />

      <template v-if="engine?.usesCredentials && needsLogin">
        <v-text-field v-model="draft.user" label="User" data-test="user-field" />
        <v-text-field
          v-model="password"
          label="Password"
          :type="showPassword ? 'text' : 'password'"
          :hint="passwordHint"
          persistent-hint
          data-test="password-field"
        >
          <!-- The icon that shows the password is a button of its own, so a
               reader can name it and a key can reach it. -->
          <template #append-inner>
            <v-btn
              :icon="showPassword ? 'mdi-eye-off' : 'mdi-eye'"
              :aria-label="showPassword ? 'Hide the password' : 'Show the password'"
              :aria-pressed="showPassword"
              size="x-small"
              variant="text"
              data-test="toggle-password"
              @click="showPassword = !showPassword"
            />
          </template>
        </v-text-field>
      </template>

      <v-text-field
        v-if="needsAccessToken"
        ref="tokenField"
        v-model="password"
        label="Access token"
        :type="showPassword ? 'text' : 'password'"
        :hint="tokenHint"
        :error="needsNewToken && password.trim() === ''"
        persistent-hint
        data-test="access-token-field"
      >
        <template #append-inner>
          <v-btn
            :icon="showPassword ? 'mdi-eye-off' : 'mdi-eye'"
            :aria-label="showPassword ? 'Hide the token' : 'Show the token'"
            :aria-pressed="showPassword"
            size="x-small"
            variant="text"
            data-test="toggle-token"
            @click="showPassword = !showPassword"
          />
        </template>
      </v-text-field>

      <v-text-field
        v-if="draft.options.mssqlAuth === MssqlAuth.EntraAzureCli"
        v-model="draft.options.azureCliPath"
        label="Path of the Azure CLI"
        placeholder="az"
        hint="Give the path when the application cannot find `az` by itself."
        persistent-hint
        data-test="azure-cli-path-field"
      />

      <v-text-field
        v-if="engine?.usesDatabase"
        v-model="draft.database"
        :label="engine.dbType === 'athena' ? 'Database (Glue)' : 'Database'"
        data-test="database-field"
      />

      <template v-if="engine?.usesAws">
        <v-text-field
          v-model="draft.options.awsRegion"
          label="AWS region"
          placeholder="us-east-1"
          data-test="aws-region-field"
        />
        <v-select
          v-model="draft.options.awsCredentialSource"
          :items="awsSourceItems"
          item-title="title"
          item-value="value"
          label="Credentials"
          data-test="aws-source-select"
        />
        <v-text-field
          v-if="draft.options.awsCredentialSource === AwsCredentialSource.Chain"
          v-model="draft.options.awsProfile"
          label="AWS profile"
          placeholder="default"
          data-test="aws-profile-field"
        />
        <template v-else>
          <v-text-field
            v-model="draft.options.awsAccessKeyId"
            label="Access key ID"
            placeholder="AKIA..."
            data-test="aws-access-key-field"
          />
          <v-text-field
            v-model="awsSecretAccessKey"
            label="Secret access key"
            type="password"
            :hint="awsSecretHint"
            persistent-hint
            data-test="aws-secret-field"
          />
          <v-text-field
            v-model="awsSessionToken"
            label="Session token"
            type="password"
            :hint="awsTokenHint"
            persistent-hint
            data-test="aws-token-field"
          />
        </template>
        <v-text-field
          v-model="draft.options.athenaWorkgroup"
          label="Workgroup"
          placeholder="primary"
          data-test="athena-workgroup-field"
        />
        <v-text-field
          v-model="draft.options.athenaOutputLocation"
          label="Output location"
          placeholder="s3://bucket/prefix/"
          data-test="athena-output-field"
        />
        <v-text-field
          v-model="draft.options.athenaCatalog"
          label="Data catalog"
          placeholder="AwsDataCatalog"
        />
        <v-switch
          v-model="draft.options.athenaResultReuse"
          label="Reuse the result of an earlier run"
          hint="A reused result scans no data, so it costs nothing."
          persistent-hint
          data-test="athena-reuse-switch"
        />
        <v-text-field
          v-if="draft.options.athenaResultReuse"
          :model-value="draft.options.athenaResultReuseMaxAgeMinutes"
          label="Reuse a result up to this age in minutes"
          type="number"
          data-test="athena-reuse-age-field"
          @update:model-value="
            (value) => (draft.options.athenaResultReuseMaxAgeMinutes = Number(value))
          "
        />
      </template>

      <v-expansion-panels variant="accordion" class="mt-2">
        <v-expansion-panel title="Advanced" data-test="advanced-panel">
          <v-expansion-panel-text>
            <div class="d-flex flex-column ga-3">
              <template v-if="engine?.usesTls">
                <v-select
                  v-model="draft.options.tlsMode"
                  :items="tlsItems"
                  item-title="title"
                  item-value="value"
                  label="Transport"
                  :hint="tlsHint"
                  persistent-hint
                  data-test="tls-select"
                />
                <v-text-field
                  v-if="draft.options.tlsMode === 'verifyFull'"
                  v-model="draft.options.caCertPath"
                  label="Certificate authority file"
                  hint="Leave this empty to use the trusted roots of the system."
                  persistent-hint
                />
              </template>

              <v-text-field
                v-if="engine?.supportsIntegratedSecurity"
                v-model="draft.options.instanceName"
                label="Named instance"
                hint="The SQL Browser service resolves the port of a named instance."
                persistent-hint
                data-test="instance-field"
              />

              <v-switch
                v-if="engine?.supportsIntegratedSecurity"
                v-model="draft.options.integratedSecurity"
                label="Use Windows Integrated Security"
                data-test="integrated-switch"
              />

              <div class="d-flex ga-2">
                <v-text-field
                  v-model.number="draft.options.connectTimeoutSecs"
                  label="Connect timeout (s)"
                  type="number"
                />
                <v-text-field
                  v-model.number="draft.options.queryTimeoutSecs"
                  label="Statement timeout (s)"
                  type="number"
                />
                <v-text-field
                  v-model.number="draft.options.maxRows"
                  label="Row limit"
                  type="number"
                />
              </div>

              <v-text-field
                v-model.number="draft.options.maxSessions"
                label="Max sessions"
                type="number"
                min="1"
                hint="The largest number of tabs that hold a session on this server at one time."
                persistent-hint
                data-test="max-sessions-field"
              />

              <v-switch v-model="draft.options.readOnly" label="Open a read-only session" />

              <v-text-field
                v-model="draft.options.applicationName"
                label="Application name"
                hint="The name the server records for this client."
                persistent-hint
              />

              <v-textarea
                v-model="draft.options.connectionUrl"
                label="Connection string"
                rows="2"
                hint="When this holds a value it replaces the fields above."
                persistent-hint
                data-test="connection-url-field"
              />

              <div class="d-flex ga-2">
                <v-text-field v-model="draft.group" label="Folder" placeholder="Connections" />
                <v-select
                  v-model="draft.color"
                  :items="colorItems"
                  item-title="title"
                  item-value="value"
                  label="Colour"
                  clearable
                />
              </div>
            </div>
          </v-expansion-panel-text>
        </v-expansion-panel>
      </v-expansion-panels>
    </v-card-text>

    <!-- The warning stands outside the part that scrolls, so a form of many
         fields cannot push it past the edge of the card. -->
    <v-alert
      v-if="problems.length > 0"
      type="warning"
      variant="tonal"
      density="compact"
      class="form-problems mx-4 mb-2"
      data-test="form-problems"
    >
      <div v-for="problem in problems" :key="problem">{{ problem }}</div>
    </v-alert>

    <v-card-actions>
      <v-btn
        :loading="connections.testing"
        prepend-icon="mdi-check-network-outline"
        text="Test"
        data-test="test-button"
        @click="test"
      />
      <v-spacer />
      <v-btn text="Cancel" data-test="cancel-button" @click="emit('close')" />
      <v-btn
        color="primary"
        variant="flat"
        text="Save"
        data-test="save-button"
        @click="saveConnection"
      />
    </v-card-actions>
  </v-card>
</template>

<script setup lang="ts">
import { computed, onMounted, nextTick, ref, watch } from 'vue'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import { useConnectionsStore, defaultPortFor, validateConnection } from '@/stores/connections'
import { useUiStore } from '@/stores/ui'
import { AwsCredentialSource, DbType, MssqlAuth, TlsMode, type SavedConnection } from '@/types/api'

const props = defineProps<{
  connection: SavedConnection
  isNew: boolean
  /** True when the stored token is too old and the user must paste a new one. */
  needsNewToken?: boolean
}>()
const emit = defineEmits<{ (event: 'close'): void; (event: 'saved', id: string): void }>()

const connections = useConnectionsStore()
const ui = useUiStore()

const draft = ref<SavedConnection>(clone(props.connection))
const password = ref(props.connection.password ?? '')
const awsSecretAccessKey = ref(props.connection.awsSecretAccessKey ?? '')
const awsSessionToken = ref(props.connection.awsSessionToken ?? '')
const showPassword = ref(false)
const tokenField = ref<{ focus: () => void } | null>(null)

const engineItems = computed(() =>
  connections.engines.map((engine) => ({ title: engine.label, value: engine.dbType })),
)

const engine = computed(() =>
  connections.engines.find((item) => item.dbType === draft.value.dbType),
)

const tlsItems = [
  { title: 'Verify the certificate (recommended)', value: TlsMode.VerifyFull },
  { title: 'Encrypt, accept any certificate', value: TlsMode.Require },
  { title: 'Encrypt when the server offers it', value: TlsMode.Prefer },
  { title: 'No encryption', value: TlsMode.Disable },
]

const awsSourceItems = [
  { title: 'The AWS tools of this machine', value: AwsCredentialSource.Chain },
  { title: 'Keys that you paste here', value: AwsCredentialSource.Keys },
]

const colorItems = [
  { title: 'Blue', value: 'primary' },
  { title: 'Green', value: 'success' },
  { title: 'Amber', value: 'warning' },
  { title: 'Red', value: 'error' },
]

const tlsHint = computed(() => {
  switch (draft.value.options.tlsMode) {
    case TlsMode.VerifyFull:
      return 'The identity of the server is checked. Use this outside a trusted network.'
    case TlsMode.Require:
      return 'The traffic is encrypted, but a server that gives a false certificate is accepted.'
    case TlsMode.Prefer:
      return 'The connection continues without encryption when the server offers none.'
    default:
      return 'The credentials and the results cross the network in clear text.'
  }
})

const authItems = [
  { title: 'SQL login', value: MssqlAuth.SqlLogin },
  { title: 'Windows Authentication', value: MssqlAuth.Integrated },
  { title: 'Microsoft Entra ID with the Azure CLI', value: MssqlAuth.EntraAzureCli },
  { title: 'Microsoft Entra ID with an access token', value: MssqlAuth.EntraAccessToken },
]

/** True while the chosen method needs a login and a password. */
const needsLogin = computed(
  () => draft.value.dbType !== DbType.Mssql || draft.value.options.mssqlAuth === MssqlAuth.SqlLogin,
)

/** True while the chosen method needs a token that the user supplies. */
const needsAccessToken = computed(
  () => draft.value.options.mssqlAuth === MssqlAuth.EntraAccessToken,
)

const authHint = computed(() => {
  switch (draft.value.options.mssqlAuth) {
    case MssqlAuth.Integrated:
      return 'The server takes the account of the user who runs the application. Windows uses its own credentials. macOS and Linux use the Kerberos ticket of the user, so run `kinit` first and give the full host name.'
    case MssqlAuth.EntraAzureCli:
      return 'The application asks the Azure CLI for a token. Run `az login` first.'
    case MssqlAuth.EntraAccessToken:
      return 'Paste a token for https://database.windows.net/. A token lives for about one hour.'
    default:
      return 'The server holds the login and the password.'
  }
})

const tokenHint = computed(() =>
  props.needsNewToken
    ? 'The stored token is too old. Paste a new token for https://database.windows.net/.'
    : 'The token is a credential. It goes to the keychain, never to a settings file.',
)

const passwordHint = computed(() =>
  props.isNew
    ? 'The password goes into the keychain of the operating system.'
    : 'Leave this empty to keep the password that is already stored.',
)

const awsSecretHint = computed(() =>
  props.isNew
    ? 'The secret access key goes into the keychain of the operating system.'
    : 'Leave this empty to keep the secret access key that is already stored.',
)

const awsTokenHint = computed(() => {
  if (draft.value.options.awsSessionTokenSet) {
    return 'Leave this empty to keep the stored token. A token lives for a limited time.'
  }
  return 'A permanent pair of keys needs no session token.'
})

const problems = computed(() => validateConnection(withSecrets()))

function clone(connection: SavedConnection): SavedConnection {
  return { ...connection, options: { ...connection.options } }
}

/** The draft with every secret that the user typed. */
function withSecrets(): SavedConnection {
  return {
    ...draft.value,
    options: { ...draft.value.options },
    password: password.value,
    awsSecretAccessKey: awsSecretAccessKey.value,
    awsSessionToken: awsSessionToken.value,
  }
}

function onEngineChange(value: DbType): void {
  draft.value.port = defaultPortFor(value)
  if (value === DbType.Sqlite || value === DbType.Athena) {
    draft.value.host = null
  } else if (!draft.value.host) {
    draft.value.host = 'localhost'
  }
}

async function chooseFile(): Promise<void> {
  try {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: 'SQLite database', extensions: ['db', 'sqlite', 'sqlite3'] }],
    })
    if (typeof path === 'string') {
      draft.value.options.filePath = path
    }
  } catch (error) {
    ui.reportError(error)
  }
}

async function test(): Promise<void> {
  await connections.test(withSecrets())
}

async function saveConnection(): Promise<void> {
  const record = withSecrets()
  if (props.needsNewToken && needsAccessToken.value && password.value.trim() === '') {
    // The stored token is too old, so an empty box cannot mean "keep it".
    ui.warn('Paste a new access token, or choose another authentication method.')
    return
  }
  if (!props.isNew && password.value === '') {
    // An empty box means that the stored password stays as it is.
    record.password = null
  }
  if (!props.isNew && awsSecretAccessKey.value === '') {
    record.awsSecretAccessKey = null
  }
  if (!props.isNew && awsSessionToken.value === '') {
    record.awsSessionToken = null
  }
  const saved = await connections.save(record)
  if (saved) {
    emit('saved', record.id)
    emit('close')
  }
}

watch(
  () => props.connection,
  (value) => {
    draft.value = clone(value)
    password.value = value.password ?? ''
    awsSecretAccessKey.value = value.awsSecretAccessKey ?? ''
    awsSessionToken.value = value.awsSessionToken ?? ''
    showPassword.value = false
    void focusToken()
  },
)

onMounted(() => {
  void focusToken()
})

/** Puts the pointer in the token box when the form asks for a new token. */
async function focusToken(): Promise<void> {
  if (!props.needsNewToken) {
    return
  }
  await nextTick()
  tokenField.value?.focus()
}
</script>

<style scoped>
/**
 * The dialog stands as `scrollable`, so the card holds the height of the
 * window and this part alone scrolls. A height of its own here would let the
 * card grow past the window, and the warning and the buttons below it would
 * then stand off the screen.
 */
.form-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* A long list of problems scrolls inside the warning, so the warning never
   takes the whole card. */
.form-problems {
  max-height: 20vh;
  overflow-y: auto;
}
</style>
