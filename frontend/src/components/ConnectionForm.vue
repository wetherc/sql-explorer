<template>
  <v-card>
    <v-card-title>
      {{ editingConnection ? 'Edit Connection' : 'New Connection' }}
    </v-card-title>
    <v-card-text>
      <v-form ref="form" v-model="valid" lazy-validation>
        <v-text-field
          v-model="connection.name"
          label="Connection Name"
          :rules="[rules.required]"
          required
        ></v-text-field>

        <v-select
          v-model="connection.dbType"
          :items="dbTypes"
          label="Database Type"
          :rules="[rules.required]"
          required
        ></v-select>

        <v-text-field
          v-model="connection.host"
          label="Host"
          :rules="[rules.required]"
          required
        ></v-text-field>

        <v-text-field
          v-model.number="connection.port"
          label="Port"
          type="number"
          :rules="[rules.required, rules.port]"
          required
        ></v-text-field>

        <v-text-field
          v-model="connection.database"
          label="Database"
        ></v-text-field>

        <v-text-field
          v-model="connection.user"
          label="User"
          :rules="[rules.required]"
          required
        ></v-text-field>

        <v-text-field
          v-model="connection.password"
          label="Password"
          type="password"
        ></v-text-field>
      </v-form>
    </v-card-text>
    <v-card-actions>
      <v-spacer></v-spacer>
      <v-btn color="error" @click="cancelConnectionForm" data-test="cancel-button">Cancel</v-btn>
      <v-btn color="success" @click.stop="save" data-test="save-button">Save</v-btn>
    </v-card-actions>
  </v-card>
</template>

<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import { useConnectionManagerStore, type SavedConnection } from '@/stores/connectionManager'
import { storeToRefs } from 'pinia'

const connectionManagerStore = useConnectionManagerStore()
const { editingConnection } = storeToRefs(connectionManagerStore)
const { saveConnection, cancelConnectionForm } = connectionManagerStore

const valid = ref(true)
const form = ref<HTMLFormElement | null>(null)

const dbTypes = ['mssql', 'mysql', 'postgres']
const defaultPorts = {
  mssql: 1433,
  mysql: 3306,
  postgres: 5432,
}

const defaultConnection: SavedConnection = {
  id: '',
  name: '',
  dbType: 'mssql',
  host: '',
  port: 1433,
  user: '',
  database: '',
  password: '',
}

const connection = ref<SavedConnection>(JSON.parse(JSON.stringify(defaultConnection)))

// Watch for changes in the selected database type to update the port
watch(() => connection.value.dbType, (newDbType) => {
  // Only update the port if we are creating a new connection
  if (!editingConnection.value) {
    connection.value.port = defaultPorts[newDbType]
  }
})

// Watch for changes in editingConnection to populate the form
watch(editingConnection, (newVal) => {
  if (newVal) {
    connection.value = JSON.parse(JSON.stringify(newVal))
  } else {
    connection.value = JSON.parse(JSON.stringify(defaultConnection))
  }
}, { immediate: true })

const rules = {
  required: (value: any) => !!value || 'Required.',
  port: (value: number) => (value > 0 && value < 65536) || 'Port must be between 1 and 65535.',
}

async function save() {
  const { valid: formValid } = await form.value!.validate()
  if (formValid) {
    await saveConnection(connection.value)
  }
}
</script>
