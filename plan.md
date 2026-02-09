# SQL Explorer - Development Plan

## 1. Project Overview

This document outlines the plan for building **SQL Explorer**, a desktop application using Vue.js for the frontend. The goal is to create a lightweight, modern, and cross-platform alternative to SQL Server Management Studio (SSMS) for developers and database administrators who need to interact with Microsoft SQL Server instances.

The application will be built using a modern web stack and wrapped in a desktop shell (like Tauri or Electron) to provide the necessary system-level access for direct database connections.

## 2. Core Technologies

- **Frontend:**
  - **Framework:** Vue 3 (Composition API)
  - **Build Tool:** Vite
  - **State Management:** Pinia
  - **Routing:** Vue Router
  - **UI Components:** A component library like PrimeVue, Vuetify, or a custom-built set with TailwindCSS for styling.
  - **SQL Editor:** Monaco Editor (the editor that powers VS Code) for a rich editing experience with syntax highlighting.

- **Backend & Desktop:**
  - **Application Shell:** **Tauri**. It's a more lightweight and secure alternative to Electron, using a Rust backend. This is critical as a browser cannot directly connect to a SQL database.
  - **Database Driver:** `tiberius` (a Rust driver for TDS, the protocol for MS SQL Server).
  - **API:** The Vue frontend will communicate with the Rust backend via Tauri's built-in IPC mechanism.

## 3. UX/UI Design Philosophy

- **Modern & Clean:** A minimalist interface that prioritizes content and reduces clutter.
- **Responsive & Performant:** A fluid layout that works well on different screen sizes and feels fast.
- **Component-Based:** A modular design with resizable panels for the database explorer, SQL editor, and results view.
- **Tabbed Interface:** Allow users to work on multiple queries simultaneously in different tabs.

---

## 4. Consolidated Project Roadmap

This section outlines the development plan in a milestone-based format.

### **Milestone 1: Core Functionality (The "Spine")**
*   **Goal:** Establish a working end-to-end connection between the Vue.js frontend and a SQL Server database, mediated by the Rust backend. This involves creating the most basic UI needed to input a query, execute it, and see the results.
*   **Status:** ✅ **Completed**

#### **Completed Tasks:**
*   **[x] M1.1 (Backend):** Add `tiberius` to `Cargo.toml`.
*   **[x] M1.2 (Backend):** Create a `connect` Tauri Command.
*   **[x] M1.3 (Backend):** Create an `execute_query` Tauri Command.
*   **[x] M1.4 (Backend):** Register `connect` and `execute_query` commands in `main.rs`.
*   **[x] M1.5 (Frontend):** Create a `ConnectionView.vue` component with a single input.
*   **[x] M1.6 (Frontend):** Implement a Pinia store for connection state management.
*   **[x] M1.7 (Frontend):** Create a `QueryView.vue` component with a `<textarea>` and `<table>`.
*   **[x] M1.8 (Frontend):** Implement the logic to call the `execute_query` command and display results.
*   **[x] M1.9 (Frontend):** Create the basic app layout to switch between `ConnectionView` and `QueryView`.

---

### **Milestone 2: Advanced Connection & UI**
*   **Goal:** Enhance the connection process with a more user-friendly UI and support for different authentication methods. Improve the core editor and results view.
*   **Status:** ✅ **Completed**

#### **Completed Tasks:**
*   **[x] M2.1 (Frontend):** Refactor `ConnectionView.vue` to use separate input fields for Server, Database, Username, and Password.
*   **[x] M2.2 (Frontend):** Add a dropdown/selector to `ConnectionView.vue` to switch between "SQL Server Authentication" and "Microsoft Entra / Integrated" authentication. The Username/Password fields should be disabled for Integrated auth.
*   **[x] M2.3 (Frontend):** Create a `connectionStringBuilder.ts` utility to dynamically build the correct ADO.NET connection string from the form fields and selected auth type.
*   **[x] M2.4 (Frontend):** Add the `monaco-editor-vue3` package (or a similar alternative) to the frontend dependencies.
*   **[x] M2.5 (Frontend):** Replace the `<textarea>` in `QueryView.vue` with the Monaco Editor component.
*   **[x] M2.6 (Frontend):** Configure the Monaco Editor for T-SQL language support (syntax highlighting).
*   **[x] M2.7 (Frontend):** Research and select a virtualized data grid component for Vue (e.g., from PrimeVue, TanStack Table, etc.).
*   **[x] M2.8 (Frontend):** Replace the simple `<table>` in `QueryView.vue` with the new virtualized grid component to improve performance with large datasets.

---

### **Milestone 3: Database Explorer & Core Features**
*   **Goal:** Implement the database object explorer and other core application features like saving connections and tabbed editing.
*   **Status:** ✅ **Completed**

#### **Completed Tasks:**

*   **[x] M3.1 & M3.2 (Backend): Database Metadata Commands**
    *   **Tauri Commands:** In `backend/src/main.rs`, create and export new `#[tauri::command]` functions: `list_databases`, `list_schemas`, `list_tables`, and `list_columns`. These will serve as the bridge to the frontend.
    *   **Database Logic (`db.rs`):**
        *   Implement the core logic for each command, which will execute SQL queries against system views.
        *   `list_databases`: `SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name`.
        *   `list_schemas`: `SELECT name FROM sys.schemas ORDER BY name`.
        *   `list_tables`: `SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = @P1 ORDER BY TABLE_NAME`.
        *   `list_columns`: `SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = @P1 AND TABLE_NAME = @P2 ORDER BY ORDINAL_POSITION`.
    *   **Connection Handling:** The backend state will hold the primary connection string. For exploring a specific database's objects, the commands will receive the target database name and establish a temporary connection to it to ensure metadata is fetched from the correct context.
    *   **Data Structures:** Define simple, serializable Rust structs for each query's return type (e.g., `struct Table { table_name: String }`).

*   **[x] M3.3, M3.4, & M3.5 (Frontend): Database Explorer UI**
    *   **Multi-Pane Layout (`App.vue`):** Modify the main app layout to use a resizable split-pane view (e.g., using CSS Flexbox and a draggable divider). This will create a side panel for the database explorer and a main area for the query editor.
    *   **Explorer Component (`DbExplorer.vue`):**
        *   Create a new component to house the explorer.
        *   Use a tree-view structure (e.g., using a component library or building a custom one with nested lists) to display the database hierarchy (Databases -> Schemas -> Tables -> Columns).
    *   **State Management (`explorerStore.ts`):**
        *   Create a new Pinia store to manage the explorer's state.
        *   The state will hold the tree data structure.
        *   Define actions (`fetchDatabases`, `fetchTables`, etc.) that call the corresponding Tauri commands using `invoke`.
        *   The store will manage loading states for asynchronous operations, allowing the UI to display spinners while fetching data. When a user clicks to expand a tree node, the store will fetch its children on-demand.

*   **[x] M3.6 & M3.7 (Frontend): Tabbed Query Editing**
    *   **Tab Management (`tabsStore.ts`):**
        *   Create a Pinia store to manage an array of open query tabs.
        *   The state will include the `tabs` array (each object containing `id`, `title`, `query` text, `results`, etc.) and the `activeTabId`.
        *   Implement actions like `addTab`, `closeTab`, and `setActiveTab`. The `addTab` action will support creating a new blank tab or a tab with a pre-filled query.
    *   **Reusable Query View:**
        *   Refactor the existing `QueryView.vue` into a generic, reusable component that represents the content of a single tab.
        *   It will receive its state (query text, results) as props and emit events for user actions like executing a query or modifying the text.
        *   A new parent component (`QueryTabs.vue`) will be responsible for rendering the tab headers and the currently active `QueryView` instance, sourcing all its data from the `tabsStore`.

*   **[x] M3.8, M3.9, & M3.10 (Backend/Frontend): Saved Connections**
    *   **Backend Commands:**
        *   Create `save_connection` and `list_connections` Tauri commands.
        *   Create a `get_connection_password` command to securely retrieve stored credentials when needed.
    *   **Secure Storage (`keyring-node` crate):**
        *   Add the `keyring-node` crate to the Rust backend.
        *   When `save_connection` is called for a connection with a password, the password will be stored securely in the operating system's secret manager (e.g., macOS Keychain, Windows Credential Manager).
        *   The rest of the connection details (server, user, database, etc.) will be stored in a simple JSON file in the app's local data directory.
    *   **Frontend UI:**
        *   Add a "Save" button to `ConnectionView.vue`.
        *   Create a `SavedConnections.vue` component (e.g., a modal or separate view) that calls `list_connections` to display saved connection profiles.
        *   When a user selects a saved connection, the form will be populated. If it uses a password, the password field will remain empty, and the `get_connection_password` command will be called transparently upon connecting.

---

### **Milestone 4: Advanced Features & Polish**
*   **Goal:** Add "quality of life" features and polish the application for a better user experience.
*   **Status:** ⏳ Not Started

#### **Technical Approach:**

*   **M4.1 & M4.2 (Frontend): Explorer Context Menu**
    *   **UI:** In `DbExplorer.vue`, add a `@contextmenu.prevent` handler on tree nodes to display a custom context menu component at the cursor's position.
    *   **Functionality:** The menu's options will depend on the node type (e.g., "New Query" for a database, "Select Top 1000 Rows" for a table).
    *   **Actions:** Selecting an action will trigger the `tabsStore`. For example, "Select Top 1000 Rows" will generate the appropriate SQL statement and call `tabsStore.addTab(generatedSql)` to open it in a new, pre-filled query tab.

*   **M4.3 & M4.5 (Backend): Enhanced Query Execution**
    *   **Multiple Result Sets:**
        *   Modify the `execute_query` logic in `db.rs`. The `tiberius` crate's `QueryStream` supports multiple result sets.
        *   Iterate through the stream using `next_resultset()` to collect all distinct results from a single query execution.
    *   **Informational Messages:** While iterating, inspect the stream for `tiberius::QueryItem::Message` items, which correspond to `PRINT` or `RAISERROR` outputs. Collect these messages.
    *   **Updated Response Structure:** Change the command's return type from a simple array to a structured object, e.g., `{ results: [ResultSet1, ResultSet2, ...], messages: ["Msg1", "Msg2", ...] }`.

*   **M4.4 & M4.6 (Frontend): Advanced Results Display**
    *   **Multi-Result UI:** In `QueryView.vue`, update the results panel to detect when the response contains multiple result sets. If so, it will render a set of tabs within the panel (e.g., "Result 1," "Result 2"). Each tab will contain a data grid for one result set.
    *   **Messages Tab:** Add a dedicated "Messages" tab to the results panel. This tab will be populated with any informational messages returned from the backend, providing feedback from `PRINT` statements or warnings.

*   **M4.7 & M4.8 (Frontend): CSV Export**
    *   **UI:** Add an "Export to CSV" button within the results panel, likely one for each result grid.
    *   **Client-Side Logic:**
        *   Implement a utility function to convert the JSON data of a result set into a CSV string. This involves creating a header row from the column names and then iterating through the rows of data.
        *   To ensure robust CSV generation (handling commas and quotes within data), consider adding a lightweight library like `papaparse`.
        *   To trigger the download, the logic will create a `Blob` with the CSV content, generate a local object URL for it, and programmatically trigger a download via a temporary `<a>` element.

