<template>
  <v-dialog v-model="model" persistent max-width="600px">
    <v-card>
      <v-card-title>
        <span class="text-h5">Database Connection</span>
      </v-card-title>
      <v-card-text>
        <v-container>
          <v-form @submit.prevent="handleConnect">
            <v-row>
              <v-col cols="12">
                <v-select
                  v-model="dbType"
                  :items="dbTypeOptions"
                  label="Database Type"
                  required
                ></v-select>
              </v-col>
              <v-col cols="12">
                <v-text-field
                  v-model="connectionString"
                  label="Connection String"
                  required
                ></v-text-field>
              </v-col>
            </v-row>
             <v-row v-if="connectionStore.errorMessage">
              <v-col cols="12">
                <v-alert type="error" :text="connectionStore.errorMessage"></v-alert>
              </v-col>
            </v-row>
          </v-form>
        </v-container>
      </v-card-text>
      <v-card-actions>
        <v-spacer></v-spacer>
        <v-btn color="blue-darken-1" variant="text" @click="model = false" :disabled="connectionStore.isConnecting">
          Close
        </v-btn>
        <v-btn color="blue-darken-1" variant="text" @click="handleConnect" :loading="connectionStore.isConnecting">
          Connect
        </v-btn>
      </v-card-actions>
    </v-card>
  </v-dialog>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { useConnectionStore } from '@/stores/connection'
import { DbType } from '@/types/db'

const model = defineModel<boolean>()
const connectionStore = useConnectionStore()

const dbType = ref<DbType>(DbType.Postgres)
const connectionString = ref('')

const dbTypeOptions = Object.values(DbType)

async function handleConnect() {
  await connectionStore.connect(connectionString.value, dbType.value)
  if (connectionStore.isConnected) {
    model.value = false
  }
}

// When the dialog opens, clear any previous error messages
watch(model, (isShowing) => {
  if (isShowing) {
    connectionStore.errorMessage = null
  }
})
</script>
