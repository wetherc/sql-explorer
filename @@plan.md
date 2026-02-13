### **SQL Explorer - UI Integration Plan**

The previous plan focused on creating the foundational components. This new plan outlines the necessary steps to integrate these components into a functional application, refactor core state management, and complete the main UI layout.

---

### **Part 1: Finalize Application Layout**

**Goal:** Assemble the created frontend components into the main application shell.

1.  **Integrate Components into `AppLayout.vue`**
    *   **Task:** In `frontend/src/layouts/AppLayout.vue`, replace the current placeholder content with the actual components.
    *   Render `<ConnectionManager />` inside the second `v-navigation-drawer` when the active view is `connections`.
    *   Render `<DbExplorer />` inside the second `v-navigation-drawer` when the active view is `explorer`.
    *   Render `<QueryTabs />` inside the `<v-main>` content area.
    *   **Files to Modify:** `frontend/src/layouts/AppLayout.vue`.

---

### **Part 2: Refactor Core Connection State**

**Goal:** Evolve the application's state management to be centered around saved connections rather than raw connection strings.

1.  **Modernize the `connection.ts` Store**
    *   **Task:** The store currently handles a primitive `connectionString`. It needs to be updated to manage the state of a full `SavedConnection` object.
    *   Add a new state property to hold the active connection object, e.g., `activeConnection: Ref<SavedConnection | null>`.
    *   The `connect` action should be refactored to accept a `SavedConnection` object. It will be responsible for calling the backend `connect` command and updating `isConnected` and `activeConnection` state.
    *   The `disconnect` action should clear the `activeConnection`.
    *   **Files to Modify:** `frontend/src/stores/connection.ts`.

2.  **Integrate `ConnectionManager` with Global State**
    *   **Task:** The `ConnectionManager` component currently attempts to connect directly. This logic must be updated to use the central `connection.ts` store.
    *   In the `connectToDatabase` method within `ConnectionManager.vue`:
        *   Remove the logic that manually constructs a connection string and calls `invoke`.
        *   Instead, call the newly refactored `connect` action from the `connection` store, passing the entire `SavedConnection` object.
        *   On successful connection, call `setActiveView('explorer')` from the `navigation` store to switch the UI to the database explorer.
    *   **Files to Modify:** `frontend/src/components/ConnectionManager.vue`.

3.  **Ensure `DbExplorer` is Connection-Aware**
    *   **Task:** The `DbExplorer` should only be visible when a database connection is active.
    *   In `AppLayout.vue`, wrap the `<DbExplorer />` component in a `v-if` that checks `connectionStore.isConnected`. This ensures the explorer panel is only shown after a successful connection.
    *   **Files to Modify:** `frontend/src/layouts/AppLayout.vue`.
