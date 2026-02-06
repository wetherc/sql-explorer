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

## 4. Feature Breakdown & Task List

### Epic 1: Connection Management

The user must be able to save and manage connections to different SQL Server instances.

- **Task 1.1: Connection Dialog UI**
  - Create a modal or dedicated page to add/edit a new connection.
  - Fields will include: Server Name/IP, Authentication Type (Windows Authentication, SQL Server Authentication), Username, Password, and an optional Connection Name.
- **Task 1.2: Save Connection**
  - Implement backend logic in Rust to receive connection details from the Vue frontend.
  - Securely store connection details on the local machine. Passwords should be encrypted using a system-level secret store (e.g., Windows Credential Manager, macOS Keychain).
- **Task 1.3: List & Select Connections**
  - Create a UI (e.g., a dropdown or a startup page) to display and select from saved connections.
- **Task 1.4: Test & Establish Connection**
  - Implement a "Test Connection" button.
  - The Rust backend will attempt to connect to the server with the provided details and return a success/error message.

### Epic 2: Database Explorer

A hierarchical tree view to navigate database objects.

- **Task 2.1: Tree View Component**
  - Implement a collapsible tree view component in the UI's main layout (e.g., a left-hand sidebar).
- **Task 2.2: Backend API for Metadata**
  - Create Rust functions exposed to the frontend to query system tables (`sys.databases`, `sys.tables`, `INFORMATION_SCHEMA.COLUMNS`, etc.).
  - Endpoints needed:
    - `list_databases(connection_id)`
    - `list_schemas(database_name)`
    - `list_tables_and_views(database_name, schema_name)`
    - `list_columns(table_name)`
- **Task 2.3: Populate Tree View**
  - On successful connection, fetch the list of databases and populate the top level of the tree.
  - Lazily load child nodes (tables, columns, etc.) as the user expands parent nodes to improve performance.
- **Task 2.4: Context Menu Actions**
  - Implement a right-click context menu on tree nodes.
  - Initial actions:
    - Table/View: `SELECT TOP 1000 Rows` (generates and opens a script in a new query tab).
    - Table: `Script as CREATE`

### Epic 3: SQL Editor

A powerful, multi-tabbed editor for writing and executing queries.

- **Task 3.1: Integrate Monaco Editor**
  - Add the Monaco Editor component to the main content area.
  - Configure it for T-SQL syntax highlighting.
- **Task 3.2: Tabbed Editing**
  - Implement a tab system so users can open multiple editor instances.
  - Each tab maintains its own state (code, connection context).
- **Task 3.3: Query Execution**
  - Create a "Run" button (and F5 shortcut).
  - On execution, send the query text from the active editor tab to the Rust backend.
  - The backend will execute the query against the currently selected database connection and stream results back.
- **Task 3.4: Status & Error Display**
  - Add a status bar to show the current connection, execution time, and row count.
  - Display error messages from the database driver in a clear, user-friendly format.

### Epic 4: Query Results Viewer

A component to display the data returned from a query.

- **Task 4.1: Data Grid Component**
  - Implement a virtualized data grid to efficiently display potentially large result sets.
  - Columns should be dynamically generated based on the query's output.
- **Task 4.2: Handle Multiple Result Sets**
  - Queries can return multiple result sets. The UI should display these in separate tabs or grids.
- **Task 4.3: Display Query Messages**
  - Create a "Messages" tab next to the "Results" grid to show output from `PRINT` statements or other server messages.
- **Task 4.4: Export Functionality (Post-MVP)**
  - Add buttons to export the contents of the results grid to formats like CSV or JSON.

---

## 5. Development Roadmap (Milestones)

- **Milestone 1: Core Functionality (The "Spine")**
  The goal of this milestone is to establish a working end-to-end connection between the Vue.js frontend and a SQL Server database, mediated by the Rust backend. This involves creating the most basic UI needed to input a query, execute it, and see the results.

  ### Decomposed Technical Tasks:

  #### 1. Backend: Establish Database Connection & Query Command

    *   **[x] Task M1.1 (Backend): Add `tiberius` to `Cargo.toml`**
      *   **Description:** Add the `tiberius` crate, which is the TDS driver for SQL Server, to the `backend/Cargo.toml` dependencies. This will enable the Rust backend to communicate with the database.
      *   **Epic Connection:** Foundational for all Epics.

  *   **[x] Task M1.2 (Backend): Create a `connect` Tauri Command**
      *   **Description:** In `backend/src/main.rs`, create a new Tauri command (e.g., `#[tauri::command] async fn connect(connection_string: String) -> Result<(), String>`). This function will take a connection string, use `tiberius` to establish a connection, and return a success or error message to the frontend.
      *   **Epic Connection:** **Task 1.4 (Test & Establish Connection)**

  *   **[x] Task M1.3 (Backend): Create an `execute_query` Tauri Command**
      *   **Description:** In `backend/src/main.rs`, create another Tauri command (e.g., `#[tauri::command] async fn execute_query(query: String) -> Result<JsonValue, String>`). This function will take a raw SQL string, execute it using the established `tiberius` client, and serialize the results into a JSON format that the frontend can easily render. For simplicity, it can manage a single, globally-shared client connection for now.
      *   **Epic Connection:** **Task 3.3 (Query Execution)**

    *   **[x] Task M1.4 (Backend): Register Commands**
      *   **Description:** In the `main` function of `backend/src/main.rs`, register the new `connect` and `execute_query` commands in the Tauri builder using `.invoke_handler(tauri::generate_handler![connect, execute_query])`.

  #### 2. Frontend: Build the Minimal User Interface

  *   **[x] Task M1.5 (Frontend): Create a Connection View**
      *   **Description:** Create a new Vue component (`ConnectionView.vue`) that contains a simple form with an input for a raw SQL Server connection string and a "Connect" button.
      *   **Epic Connection:** A simplified version of **Task 1.1 (Connection Dialog UI)**. We use a raw connection string for now instead of a full dialog with separate fields to speed up initial development.

  *   **Task M1.6 (Frontend): Implement Connection State Management**
      *   **Description:** Create a Pinia store (`connectionStore.ts`) to manage the application's connection state (e.g., `isConnected`, `errorMessage`). When the "Connect" button is clicked, call the `connect` backend command using Tauri's `invoke` API and update the store based on the result.
      *   **Epic Connection:** **Task 1.4 (Test & Establish Connection)**

  *   **Task M1.7 (Frontend): Create a Query View**
      *   **Description:** Create a `QueryView.vue` component that is shown only when `isConnected` is true. This component will contain:
          1.  A basic `<textarea>` for SQL query input.
          2.  An "Execute" button.
          3.  A simple HTML `<table>` to display query results.
          4.  A pre-formatted block to display any error messages.
      *   **Epic Connection:** A minimal implementation of **Task 3.3 (Query Execution)** and **Task 4.1 (Data Grid Component)**.

  *   **Task M1.8 (Frontend): Implement Query Execution Logic**
      *   **Description:** In `QueryView.vue`, when the "Execute" button is clicked, take the text from the `<textarea>` and call the `execute_query` backend command.
      *   On success, parse the returned JSON data and dynamically generate the rows and columns for the HTML `<table>`.
      *   On error, display the error message.
      *   **Epic Connection:** **Task 3.3 (Query Execution)** and **Task 3.4 (Status & Error Display)**.

  *   **Task M1.9 (Frontend): Basic App Layout**
      *   **Description:** Modify `App.vue` to conditionally render `ConnectionView.vue` or `QueryView.vue` based on the `isConnected` state from the Pinia store. This provides the basic single-page application flow for this milestone.

- **Milestone 2: The Editor & Explorer**
  1. Replace the `textarea` with the Monaco Editor component.
  2. Build the database explorer tree view UI.
  3. Implement the backend metadata queries and connect them to the explorer to display the database/table hierarchy.
  4. Implement the tabbed interface for the editor.

- **Milestone 3: The Results Grid & UX Refinements**
  1. Replace the basic HTML table with a robust, virtualized data grid.
  2. Implement resizable panels for the explorer, editor, and results view.
  3. Add the "Messages" tab for server output.
  4. Refine the connection management UI and implement secure credential storage.

- **Milestone 4: Advanced Features & Polish**
  1. Implement context-menu actions in the database explorer.
  2. Add result set export functionality (CSV/JSON).
  3. Begin work on IntelliSense/autocomplete in the editor by feeding it schema information from the backend.
  4. General bug fixing, performance tuning, and UI polishing.
