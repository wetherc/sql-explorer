<template>
  <v-app>
    <v-navigation-drawer
      permanent
      width="64"
      class="pa-2"
    >
      <v-btn
        icon="mdi-lan-connect"
        class="mb-2"
        @click="setActiveView('connections')"
      ></v-btn>
      <v-btn
        icon="mdi-database-search"
        @click="setActiveView('explorer')"
      ></v-btn>
    </v-navigation-drawer>

    <v-navigation-drawer
      permanent
      width="300"
      resizable
    >
      <ConnectionManager v-if="activeView === 'connections'" />
      <DbExplorer v-if="activeView === 'explorer' && isConnected" />
    </v-navigation-drawer>

    <v-main>
      <QueryTabs />
    </v-main>
  </v-app>
</template>

<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { useNavigationStore } from '@/stores/navigation'
import { useConnectionStore } from '@/stores/connection'
import ConnectionManager from '@/components/ConnectionManager.vue'
import DbExplorer from '@/components/DbExplorer.vue'
import QueryTabs from '@/components/QueryTabs.vue'

const navigationStore = useNavigationStore()
const { activeView } = storeToRefs(navigationStore)
const { setActiveView } = navigationStore

const connectionStore = useConnectionStore()
const { isConnected } = storeToRefs(connectionStore)
</script>
