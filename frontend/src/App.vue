<template>
  <v-app>
    <v-app-bar app density="compact">
      <v-toolbar-title>SQL Explorer</v-toolbar-title>
      <v-spacer></v-spacer>
      <v-btn @click="isDialogVisible = true">Connect</v-btn>
    </v-app-bar>

    <v-navigation-drawer app permanent>
      <DbExplorer />
    </v-navigation-drawer>

    <v-main>
      <QueryTabs />
    </v-main>

    <ConnectionDialog v-model="isDialogVisible" />
  </v-app>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import ConnectionDialog from './components/ConnectionDialog.vue'
import DbExplorer from './components/DbExplorer.vue'
import QueryTabs from './components/QueryTabs.vue'

const isDialogVisible = ref(false)
</script>
