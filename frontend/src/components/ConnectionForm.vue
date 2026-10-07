<template>
  <v-card>
    <v-card-title class="text-subtitle-1">
      {{ isNew ? 'New connection' : `Edit ${draft.name || 'connection'}` }}
    </v-card-title>

    <!-- The fields stay read-only while a test runs, so the answer of the
         test belongs to the values the user sees. -->
    <v-form :disabled="testing" class="form-scroll" @submit.prevent>
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

        <v-text-field
          v-model="draft.name"
          label="Name"
          :error-messages="fieldProblem('name')"
          data-test="name-field"
        />

        <template v-if="engine?.usesHost">
          <div class="d-flex ga-2">
            <v-text-field
              v-model="draft.host"
              label="Host"
              class="flex-grow-1"
              :error-messages="fieldProblem('host')"
              data-test="host-field"
            />
            <v-text-field
              v-model.number="draft.port"
              label="Port"
              type="number"
              style="max-width: 130px"
              :disabled="usesInstance"
              :hint="usesInstance ? 'SQL Browser gives the port' : undefined"
              :persistent-hint="usesInstance"
              :error-messages="fieldProblem('port')"
              data-test="port-field"
            />
          </div>
        </template>

        <v-text-field
          v-if="engine?.usesFile"
          v-model="draft.options.filePath"
          label="Database file"
          :error-messages="fieldProblem('filePath')"
          data-test="file-field"
        >
          <template #append-inner>
            <v-btn
              icon="mdi-folder-open-outline"
              size="x-small"
              aria-label="Browse for file"
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
          <v-text-field
            v-model="draft.user"
            label="User"
            :error-messages="fieldProblem('user')"
            data-test="user-field"
          />
          <v-text-field
            v-model="password"
            label="Password"
            :type="showPassword ? 'text' : 'password'"
            :hint="passwordHint"
            persistent-hint
            :error-messages="fieldProblem('password')"
            data-test="password-field"
          >
            <!-- The icon that shows the password is a button of its own, so a
               reader can name it and a key can reach it. -->
            <template #append-inner>
              <v-btn
                :icon="showPassword ? 'mdi-eye-off' : 'mdi-eye'"
                :aria-label="showPassword ? 'Hide password' : 'Show password'"
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
              :aria-label="showPassword ? 'Hide token' : 'Show token'"
              :aria-pressed="showPassword"
              size="x-small"
              variant="text"
              data-test="toggle-token"
              @click="showPassword = !showPassword"
            />
          </template>
        </v-text-field>

        <v-text-field
          v-if="usesAzureCli"
          v-model="draft.options.azureCliPath"
          label="Azure CLI path"
          placeholder="az"
          hint="Only needed if the app can't find `az` on its own."
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
            :error-messages="fieldProblem('awsRegion')"
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
              :error-messages="fieldProblem('awsAccessKeyId')"
              label="Access key ID"
              placeholder="AKIA..."
              data-test="aws-access-key-field"
            />
            <v-text-field
              v-model="awsSecretAccessKey"
              label="Secret access key"
              type="password"
              :hint="secretHint"
              persistent-hint
              :error-messages="fieldProblem('awsSecretAccessKey')"
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
            :error-messages="fieldProblem('athenaWorkgroup')"
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
            label="Reuse earlier query results"
            hint="Reused results don't scan any data, so they're free."
            persistent-hint
            data-test="athena-reuse-switch"
          />
          <v-text-field
            v-if="draft.options.athenaResultReuse"
            :model-value="draft.options.athenaResultReuseMaxAgeMinutes"
            :error-messages="fieldProblem('athenaResultReuseMaxAgeMinutes')"
            label="Maximum result age (minutes)"
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
                    hint="Leave empty to use the system's trusted roots."
                    persistent-hint
                  />
                </template>

                <v-text-field
                  v-if="engine?.supportsIntegratedSecurity"
                  v-model="draft.options.instanceName"
                  label="Named instance"
                  hint="SQL Browser finds the port for a named instance."
                  persistent-hint
                  data-test="instance-field"
                />

                <div class="d-flex ga-2">
                  <v-text-field
                    v-model.number="draft.options.connectTimeoutSecs"
                    :error-messages="fieldProblem('connectTimeoutSecs')"
                    label="Connect timeout (seconds)"
                    type="number"
                  />
                  <v-text-field
                    v-model.number="draft.options.queryTimeoutSecs"
                    :error-messages="fieldProblem('queryTimeoutSecs')"
                    label="Statement timeout (seconds)"
                    type="number"
                  />
                  <v-text-field
                    v-model.number="draft.options.maxRows"
                    :error-messages="fieldProblem('maxRows')"
                    label="Row limit"
                    type="number"
                    hint="A query stops at this limit or the row limit in Settings, whichever is lower."
                    persistent-hint
                  />
                </div>

                <v-text-field
                  v-model.number="draft.options.maxSessions"
                  :error-messages="fieldProblem('maxSessions')"
                  label="Max sessions"
                  type="number"
                  min="1"
                  hint="How many tabs can have their own session on this server at once."
                  persistent-hint
                  data-test="max-sessions-field"
                />

                <v-switch
                  v-if="readOnlySwitch"
                  v-model="draft.options.readOnly"
                  :label="readOnlySwitch.label"
                  :hint="readOnlySwitch.hint"
                  persistent-hint
                  data-test="read-only-switch"
                />

                <v-text-field
                  v-model="draft.options.applicationName"
                  label="Application name"
                  hint="The client name the server sees."
                  persistent-hint
                />

                <v-textarea
                  v-model="draft.options.connectionUrl"
                  label="Connection string"
                  rows="2"
                  hint="If set, this overrides the host, port, and database. The Transport and Authentication settings apply when the string doesn't set them. Enter the password in the Password field, not in the string."
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
    </v-form>

    <!-- The warnings stand outside the part that scrolls, so a form of many
         fields cannot push them past the edge of the card. -->
    <v-alert
      v-if="!connections.passwordsPersist"
      type="info"
      variant="tonal"
      density="compact"
      class="form-problems mx-4 mb-2"
      data-test="keychain-note"
    >
      The system keychain isn't available. Passwords and keys you save here last until the app
      closes, and you'll be asked for them again next time.
    </v-alert>
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
    <v-alert
      v-if="testResult"
      :type="testResult.ok ? 'success' : 'error'"
      variant="tonal"
      density="compact"
      closable
      class="form-problems mx-4 mb-2"
      data-test="test-result"
      @click:close="testResult = null"
    >
      <div>{{ testResult.message }}</div>
      <div v-if="testResult.detail" class="test-detail">{{ testResult.detail }}</div>
    </v-alert>

    <v-card-actions>
      <v-btn
        :loading="testing"
        prepend-icon="mdi-check-network-outline"
        text="Test"
        data-test="test-button"
        @click="test"
      />
      <v-spacer />
      <v-btn text="Cancel" data-test="cancel-button" @click="cancel" />
      <v-btn
        color="primary"
        variant="flat"
        text="Save"
        :disabled="testing"
        data-test="save-button"
        @click="saveConnection"
      />
    </v-card-actions>

    <ConfirmDialog
      :open="confirmingDiscard"
      title="Discard your changes?"
      message="The changes to this connection haven't been saved."
      confirm-text="Discard"
      danger
      @confirm="discard"
      @cancel="confirmingDiscard = false"
    />
  </v-card>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, nextTick, ref, watch } from 'vue'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import ConfirmDialog from './ConfirmDialog.vue'
import { api } from '@/lib/api'
import { toErrorPayload } from '@/lib/errors'
import { useConnectionsStore, defaultPortFor, validateConnection } from '@/stores/connections'
import { useUiStore } from '@/stores/ui'
import { AwsCredentialSource, DbType, MssqlAuth, TlsMode, type SavedConnection } from '@/types/api'

const props = defineProps<{
  connection: SavedConnection
  isNew: boolean
  /** True when the stored token is too old and the user must paste a new one. */
  needsNewToken?: boolean
  /** True when the record is a copy of another one, whose secrets stay behind. */
  isCopy?: boolean
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
  { title: 'Verify certificate (recommended)', value: TlsMode.VerifyFull },
  { title: 'Encrypt, accept any certificate', value: TlsMode.Require },
  { title: 'Encrypt if the server supports it', value: TlsMode.Prefer },
  { title: 'No encryption', value: TlsMode.Disable },
]

const awsSourceItems = [
  { title: 'AWS profile', value: AwsCredentialSource.Chain },
  { title: 'Access keys', value: AwsCredentialSource.Keys },
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
      return "Checks the server's identity. Use this outside a trusted network."
    case TlsMode.Require:
      return 'Traffic is encrypted, but unverified certificates are accepted.'
    case TlsMode.Prefer:
      return "Falls back to an unencrypted connection if the server doesn't support encryption."
    default:
      return 'Credentials and results are sent over the network in plain text.'
  }
})

const authItems = [
  { title: 'SQL login', value: MssqlAuth.SqlLogin },
  { title: 'Windows Authentication', value: MssqlAuth.Integrated },
  { title: 'Microsoft Entra ID (Azure CLI)', value: MssqlAuth.EntraAzureCli },
  { title: 'Microsoft Entra ID (access token)', value: MssqlAuth.EntraAccessToken },
]

/** True while the chosen method needs a login and a password. */
const needsLogin = computed(
  () => draft.value.dbType !== DbType.Mssql || draft.value.options.mssqlAuth === MssqlAuth.SqlLogin,
)

/**
 * True when the record is a MS SQL Server record that uses the method. The
 * method stays in the options after a change of the engine, so a check of
 * the method alone would show its fields for another engine.
 */
function usesMssqlMethod(method: MssqlAuth): boolean {
  return draft.value.dbType === DbType.Mssql && draft.value.options.mssqlAuth === method
}

/** True while the chosen method needs a token that the user supplies. */
const needsAccessToken = computed(() => usesMssqlMethod(MssqlAuth.EntraAccessToken))

/** True while the chosen method asks the Azure CLI for a token. */
const usesAzureCli = computed(() => usesMssqlMethod(MssqlAuth.EntraAzureCli))

/** True while the Password box or the Access token box is in the form. */
const usesSecretBox = computed(() => needsLogin.value || needsAccessToken.value)

const authHint = computed(() => {
  switch (draft.value.options.mssqlAuth) {
    case MssqlAuth.Integrated:
      return 'On macOS and Linux this uses your Kerberos ticket, so you may need to run `kinit` first.'
    case MssqlAuth.EntraAzureCli:
      return 'Gets a token from the Azure CLI. Run `az login` first.'
    case MssqlAuth.EntraAccessToken:
      return 'Paste a token for https://database.windows.net/. Tokens expire after about an hour.'
    default:
      return 'A SQL Server login and password.'
  }
})

const tokenHint = computed(() =>
  props.needsNewToken
    ? 'The saved token has expired. Paste a new token for https://database.windows.net/.'
    : 'Tokens are saved in the keychain, never in a settings file.',
)

const passwordHint = computed(() => {
  if (props.isCopy) {
    return 'Enter the password again for the copy.'
  }
  if (targetChanged.value) {
    return 'Enter the password again because the server changed.'
  }
  return props.isNew
    ? 'Your password is saved in the system keychain.'
    : 'Leave empty to keep the saved password.'
})

/** The hint of the AWS secret box, for a copy or a record that points elsewhere. */
const secretHint = computed(() => {
  if (props.isCopy) {
    return 'Enter the secret access key again for the copy.'
  }
  return targetChanged.value ? 'Enter the secret access key again because the server changed.' : ''
})

/**
 * True when a saved record points at another server. The backend drops
 * the stored secret in that case, so the old password is never sent to a
 * host that the user did not save it for, and the form asks for it again.
 */
const targetChanged = computed(() => {
  if (props.isNew) {
    return false
  }
  // The backend compares the same fields, without the spaces at their ends,
  // and a blank field is the same as a missing one.
  const target = (connection: typeof draft.value): string =>
    JSON.stringify([
      connection.dbType,
      connection.port ?? null,
      ...[
        connection.host,
        connection.user,
        connection.options.instanceName,
        connection.options.connectionUrl,
        connection.options.filePath,
        connection.options.awsRegion,
        connection.options.awsAccessKeyId,
      ].map((field) => (field ?? '').trim()),
    ])
  return target(props.connection) !== target(draft.value)
})

/**
 * True while a named instance is set. SQL Browser then gives the port and
 * the backend leaves the port of the record out, so the box is shut.
 */
const usesInstance = computed(
  () =>
    draft.value.dbType === DbType.Mssql && (draft.value.options.instanceName ?? '').trim() !== '',
)

const awsTokenHint = computed(() => {
  return 'Not needed for long-term IAM keys.'
})

/** The label and the hint of the read-only switch, or null to hide it. */
const readOnlySwitch = computed(() => {
  switch (engine.value?.readOnly) {
    case 'session':
      return {
        label: 'Read-only session',
        hint:
          draft.value.dbType === DbType.Sqlite
            ? 'Opens the file as read-only.'
            : 'The server rejects writes. A SET statement in the session can turn this off.',
      }
    case 'intent':
      return {
        label: 'Read-only replica',
        hint: 'Connects to a readable secondary in an availability group. A primary or standalone server still accepts writes.',
      }
    default:
      return null
  }
})

/** The problems of the form, by the name of the field they belong to. */
const fieldProblems = computed<Record<string, string>>(() => {
  const byField = { ...validateConnection(withSecrets()) }
  if (targetChanged.value) {
    if (usesSecretBox.value && password.value === '') {
      byField.password = 'Enter the password again because the server changed.'
    }
    if (
      draft.value.dbType === DbType.Athena &&
      draft.value.options.awsCredentialSource === AwsCredentialSource.Keys &&
      awsSecretAccessKey.value === ''
    ) {
      byField.awsSecretAccessKey = 'Enter the secret access key again because the server changed.'
    }
  }
  return byField
})

const problems = computed(() => [...new Set(Object.values(fieldProblems.value))])

/** The problem of one field, which the field shows under itself. */
function fieldProblem(field: string): string[] {
  const problem = fieldProblems.value[field]
  return problem ? [problem] : []
}

/** True while the form waits for the answer of a test. */
const testing = ref(false)
/** The answer of the last test, which the form shows above its buttons. */
const testResult = ref<{ ok: boolean; message: string; detail: string | null } | null>(null)
/** False once the form has gone, so a late answer of a test is dropped. */
let mounted = true
onBeforeUnmount(() => {
  mounted = false
})

const confirmingDiscard = ref(false)
/**
 * The record as it stood when the form opened, to tell whether it changed.
 * The form takes it after its first watchers ran, because they can clear
 * the port of a named instance.
 */
let opened = ''
onMounted(() => {
  opened = snapshot()
})

function snapshot(): string {
  return JSON.stringify([
    draft.value,
    password.value,
    awsSecretAccessKey.value,
    awsSessionToken.value,
  ])
}

/** Closes the form, and asks first when it has changes that are not saved. */
function cancel(): void {
  if (snapshot() !== opened) {
    confirmingDiscard.value = true
    return
  }
  emit('close')
}

function discard(): void {
  confirmingDiscard.value = false
  emit('close')
}

defineExpose({ cancel })

function clone(connection: SavedConnection): SavedConnection {
  return { ...connection, options: { ...connection.options } }
}

/** The draft with every secret that the user typed. */
function withSecrets(): SavedConnection {
  const options = { ...draft.value.options }
  // The field hides for the other modes, so a path typed earlier is not
  // saved with a mode that does not use it.
  if (options.tlsMode !== TlsMode.VerifyFull) {
    options.caCertPath = null
  }
  if (!usesAzureCli.value) {
    options.azureCliPath = null
  }
  return {
    ...draft.value,
    options,
    // A method without a secret sends an empty text, which takes a password
    // or a token that an earlier method stored away from the keychain.
    password: usesSecretBox.value ? password.value : '',
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

/**
 * The record to send to the backend. On a saved connection an empty box means
 * that the stored secret stays as it is, so the field goes out absent and the
 * backend fills it from the keychain. An empty text would instead mean an
 * empty secret.
 */
function recordToSend(): SavedConnection {
  const record = withSecrets()
  if (props.isNew) {
    return record
  }
  if (usesSecretBox.value && password.value === '') {
    record.password = null
  }
  if (awsSecretAccessKey.value === '') {
    record.awsSecretAccessKey = null
  }
  if (awsSessionToken.value === '') {
    record.awsSessionToken = null
  }
  return record
}

/**
 * Tests the record and writes the answer in the form, beside the fields it
 * belongs to. A notice in the corner would leave the form and go by itself.
 */
async function test(): Promise<void> {
  if (problems.value.length > 0) {
    testResult.value = null
    return
  }
  testing.value = true
  testResult.value = null
  try {
    const message = await api.testConnection(recordToSend())
    if (mounted) {
      testResult.value = { ok: true, message, detail: null }
    }
  } catch (error) {
    if (mounted) {
      const payload = toErrorPayload(error)
      testResult.value = { ok: false, message: payload.message, detail: payload.detail ?? null }
    }
  } finally {
    testing.value = false
  }
}

async function saveConnection(): Promise<void> {
  if (props.needsNewToken && needsAccessToken.value && password.value.trim() === '') {
    // The stored token is too old, so an empty box cannot mean "keep it".
    ui.warn('Paste a new access token or pick a different authentication method.')
    return
  }
  if (problems.value.length > 0) {
    // The form already lists each problem, so no notice repeats them.
    return
  }
  const record = recordToSend()
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
    testResult.value = null
    void nextTick(() => {
      opened = snapshot()
    })
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

/* The warning shows every problem at once, whatever the number of them, so
   it never scrolls. It keeps its whole height while the fields above it give
   way, because the fields already scroll. */
.form-problems {
  flex: 0 0 auto;
}

/* The form takes the room between the title and the buttons and scrolls
   inside it, as the card text does. */
.form-scroll {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
}

.test-detail {
  font-size: var(--app-text-sm);
  white-space: pre-wrap;
}
</style>
