# SQL Explorer UI Refinement Plan

This document outlines the plan to address the gaps between the current UI implementation and the wireframe design. The goal is to evolve the primitive first pass into a more robust, user-friendly, and feature-rich application.

---

## Part 1: Application Shell and Core Navigation

This is the foundation for the new UI. It establishes the main layout and navigation structure as depicted in the wireframe.

1.  **Create the Main Application Layout (`AppLayout.vue`)**
    *   This component will be the new root of the UI, replacing the current logic in `App.vue`.
    *   It will be composed of three main sections:
        *   A `v-navigation-drawer` for the "Application Switcher Sidebar" on the far left. This will be permanent and narrow.
        *   A `v-navigation-drawer` for the "Active Panel" to its right.
        *   The main content area, which will contain the `QueryTabs.vue` component.
    *   **File to be created:** `frontend/src/layouts/AppLayout.vue`

2.  **Implement the Application Switcher Sidebar**
    *   The sidebar will contain `v-btn` or `v-list-item` elements with icons to switch between "applications" or views.
    *   **Initial views:**
        *   **Connections:** (Icon: `mdi-lan-connect`) Manages saved database connections.
        *   **Explorer:** (Icon: `mdi-database-search`) The database object treeview for the active connection.
    *   Create a new Pinia store (`navigation.ts`) to manage the currently active view.
    *   **Files to be created/modified:** `frontend/src/layouts/AppLayout.vue`, `frontend/src/stores/navigation.ts`.

3.  **Implement the Dynamic "Active Panel"**
    *   This panel's content will be driven by the state in the `navigation.ts` store.
    *   Use a dynamic `<component :is="activePanelComponent">` or `v-if`/`v-else-if` blocks to render the correct component.
    *   When "Connections" is active, it will render a new `ConnectionManager.vue` component.
    *   When "Explorer" is active, it will render the existing `DbExplorer.vue` component.
    *   **Files to be modified:** `frontend/src/layouts/AppLayout.vue`.

---

## Part 2: Advanced Connection Management

This part focuses on building a user-friendly system for creating, saving, and managing database connections, replacing the primitive connection string dialog.

1.  **Backend Support for Saved Connections**
    *   **Add `tauri-plugin-store`:** Modify `backend/Cargo.toml` and `backend/src/main.rs` to add and register the store plugin. This provides persistent key-value storage.
    *   **Define `SavedConnection` Struct:** In a new `backend/src/storage.rs` file, define a Rust struct `SavedConnection` that is serializable and contains `id`, `name`, `db_type`, and all necessary connection fields (host, user, port, etc.).
    *   **Create Backend Commands:**
        *   `save_connection(connection: SavedConnection)`: Saves or updates a connection in the store.
        *   `get_connections()`: Returns a `Vec<SavedConnection>` of all saved connections.
        *   `delete_connection(id: String)`: Removes a connection from the store.
    *   **Files to be created/modified:** `backend/Cargo.toml`, `backend/src/main.rs`, `backend/src/storage.rs`, `backend/src/commands.rs`.

2.  **Frontend Connection Manager UI**
    *   **Create `ConnectionManager.vue`:**
        *   This component will be the main UI for connection management.
        *   It will fetch and display a list of saved connections from the new backend command.
        *   Each item in the list will show the connection name and have "Connect", "Edit", and "Delete" buttons.
    *   **Create `ConnectionForm.vue`:**
        *   This will be a form-based dialog for creating and editing connections. It will replace the functionality of `ConnectionDialog.vue`.
        *   It will contain `v-text-field` and `v-select` inputs for Name, Database Type, Host, Port, User, Password, etc.
        *   Use conditional rendering (`v-if`) to show/hide fields that are only relevant to specific database types (e.g., hide "Schema" for MySQL).
    *   **Create `connectionManager.ts` Store:** A new Pinia store to manage the state of saved connections, the form dialog visibility, and the connection being edited.
    *   **Files to be created/modified:** `frontend/src/components/ConnectionManager.vue`, `frontend/src/components/ConnectionForm.vue`, `frontend/src/stores/connectionManager.ts`.

---

## Part 3: UI/UX Refinements and Integration

This part focuses on polishing the UI and integrating the new features into the main application layout.

1.  **Refactor `App.vue`**
    *   The existing logic in `App.vue` will be gutted.
    *   It will now simply render the new `AppLayout.vue` component.

2.  **Improve "New Query" Button**
    *   In `QueryTabs.vue` or a new top-level toolbar component, replace the large floating action button.
    *   Use a standard `v-btn` with an icon (`mdi-plus`) and potentially text ("New Query"). Position it in a more conventional location, like the top of the query tabs bar.
    *   **Files to be modified:** `frontend/src/App.vue`, `frontend/src/components/QueryTabs.vue`.

3.  **Integrate `DbExplorer`**
    *   Ensure `DbExplorer.vue` is correctly rendered within the "Active Panel" only when the "Explorer" view is active and a database connection is established.
    *   The explorer should be cleared or hidden when the connection is closed.

4.  **Update State Management**
    *   Refactor the existing `connection.ts` store. The concept of `isConnected` and `dbType` will now be tied to selecting and connecting to a *saved connection*.
    *   The `connect` action will take a `SavedConnection` object, construct the connection string, and then call the backend.
    *   The `disconnect` action will clear the active connection state and hide the `DbExplorer`.
    *   **Files to be modified:** `frontend/src/stores/connection.ts`.
