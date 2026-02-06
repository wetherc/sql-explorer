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
*   **Status:** 🟡 **In Progress**

#### **Outstanding Tasks:**
*   **[x] M2.1 (Frontend):** Refactor `ConnectionView.vue` to use separate input fields for Server, Database, Username, and Password.
*   **[x] M2.2 (Frontend):** Add a dropdown/selector to `ConnectionView.vue` to switch between "SQL Server Authentication" and "Microsoft Entra / Integrated" authentication. The Username/Password fields should be disabled for Integrated auth.
*   **[x] M2.3 (Frontend):** Create a `connectionStringBuilder.ts` utility to dynamically build the correct ADO.NET connection string from the form fields and selected auth type.
*   **[x] M2.4 (Frontend):** Add the `monaco-editor-vue3` package (or a similar alternative) to the frontend dependencies.
*   **M2.5 (Frontend):** Replace the `<textarea>` in `QueryView.vue` with the Monaco Editor component.
*   **M2.6 (Frontend):** Configure the Monaco Editor for T-SQL language support (syntax highlighting).
*   **M2.7 (Frontend):** Research and select a virtualized data grid component for Vue (e.g., from PrimeVue, TanStack Table, etc.).
*   **M2.8 (Frontend):** Replace the simple `<table>` in `QueryView.vue` with the new virtualized grid component to improve performance with large datasets.

---

### **Milestone 3: Database Explorer & Core Features**
*   **Goal:** Implement the database object explorer and other core application features like saving connections and tabbed editing.
*   **Status:** ⏳ Not Started

#### **Outstanding Tasks:**
*   **M3.1 (Backend):** Create new Tauri commands: `list_databases`, `list_schemas`, `list_tables`, and `list_columns`.
*   **M3.2 (Backend):** Implement the database logic for the new metadata commands in `db.rs` by querying system views (e.g., `sys.databases`, `INFORMATION_SCHEMA.TABLES`).
*   **M3.3 (Frontend):** Create a new `DbExplorer.vue` component containing a basic tree-view structure.
*   **M3.4 (Frontend):** Create a new `explorerStore.ts` Pinia store to manage the tree state and fetch data from the backend metadata commands.
*   **M3.5 (Frontend):** Implement a multi-pane layout in `App.vue` to integrate the `DbExplorer.vue` component alongside the `QueryView`.
*   **M3.6 (Frontend):** Create a `tabsStore.ts` Pinia store to manage an array of open query tabs, where each tab has its own query text, results, and state.
*   **M3.7 (Frontend):** Refactor `QueryView.vue` into a reusable component that represents a single tab's content, with its state driven by the `tabsStore`.
*   **M3.8 (Backend):** Create `save_connection` and `list_connections` Tauri commands.
*   **M3.9 (Backend):** Implement logic to save connection details to a local JSON file. For password security, integrate the `keyring` crate to use the OS secret store.
*   **M3.10 (Frontend):** Add a "Save" button to the connection dialog and create a new view or modal to list, select, and manage saved connections.

---

### **Milestone 4: Advanced Features & Polish**
*   **Goal:** Add "quality of life" features and polish the application for a better user experience.
*   **Status:** ⏳ Not Started

#### **Outstanding Tasks:**
*   **M4.1 (Frontend):** Add a right-click context menu to nodes in the `DbExplorer.vue` tree.
*   **M4.2 (Frontend):** Implement context menu actions, such as "New Query" or "Select Top 1000 Rows," which will open and pre-fill a new query tab.
*   **M4.3 (Backend):** Update `db_execute_query` to handle multiple result sets from a single query execution and update the JSON response structure accordingly.
*   **M4.4 (Frontend):** Update the `QueryView` results panel to display multiple result sets, for example, using tabs within the panel.
*   **M4.5 (Backend):** Update `db_execute_query` to capture and return informational messages (e.g., from `PRINT` statements) in the JSON response.
*   **M4.6 (Frontend):** Create a "Messages" tab in the results panel to display these informational messages.
*   **M4.7 (Frontend):** Add an "Export to CSV" button to the results panel.
*   **M4.8 (Frontend):** Implement client-side logic to convert the JSON result data into a CSV string and trigger a file download.

