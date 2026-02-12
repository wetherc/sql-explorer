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
    >
      <div v-if="activeView === 'connections'">
        <!-- ConnectionManager will go here -->
        <p class="pa-4">Connection Manager</p>
      </div>
      <div v-if="activeView === 'explorer'">
        <!-- DbExplorer will go here -->
        <p class="pa-4">DB Explorer</p>
      </div>
    </v-navigation-drawer>

    <v-main>
      <!-- QueryTabs will go here -->
      <p class="pa-4">Main Content</p>
    </v-main>
  </v-app>
</template>

<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { useNavigationStore } from '@/stores/navigation'

const navigationStore = useNavigationStore()
const { activeView } = storeToRefs(navigationStore)
const { setActiveView } = navigationStore
</script>
