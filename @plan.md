# SQL Explorer Frontend - Design Philosophy & Technical Plan

## Part 1: Design and User Experience (UX) Philosophy

This document establishes a clear and unified vision for the SQL Explorer's frontend, centered on the **Vuetify** component framework and Material Design principles.

### Core Principles

1.  **Clarity & Focus:** The user interface will be clean, uncluttered, and highly intuitive. The primary goal is to help users focus on their data and queries, not on navigating complex UI. We will achieve this through generous use of whitespace, clear typography, and a consistent, predictable layout. The Material Design system provides a robust foundation for this principle.

2.  **Efficiency & Flow:** The application must feel fast and responsive. Common user workflows—connecting to a database, exploring objects, writing and executing queries, and viewing results—will be streamlined to require a minimum number of clicks and cognitive effort. Keyboard accessibility and shortcuts will be prioritized for power users, enabling them to perform actions without leaving the keyboard.

3.  **Accessibility (A11y):** The application must be usable by the widest possible audience. We will adhere to WCAG 2.1 standards by:
    *   Ensuring sufficient color contrast for all text and UI elements.
    *   Guaranteeing full keyboard navigability for all interactive components.
    *   Providing clear labels and ARIA attributes for screen reader support.
    *   Vuetify's components are built with accessibility in mind, which provides a strong starting point for this commitment.

4.  **Consistency & Predictability:** The UI will be consistent throughout the application. A button, icon, or menu will always look and behave in a predictable manner. This reduces cognitive load and makes the application easy to master. By strictly adhering to the Material Design component system provided by Vuetify, we ensure a cohesive and predictable user experience.

---

## Part 2: UI Architecture and Interaction Patterns

Based on the project wireframe, the application will be structured around a flexible three-panel layout, providing clear separation between application modes, contextual tools, and the main workspace.

### Major UI Components

1.  **Application Layout (`App.vue`)**
    *   **Root:** The entire application will be wrapped in a `<v-app>` component.
    *   **Application Switcher (Far Left):** A thin, rail-style navigation drawer (`<v-navigation-drawer :rail="true">`) will be permanently docked on the far left. It will contain a `<v-list>` of icons (e.g., `<v-icon>mdi-database</v-icon>`, `<v-icon>mdi-lan-connect</v-icon>`) that allows the user to switch between the primary application modes, such as "Database Explorer" and "Connection Manager".
    *   **Active Panel (Middle Left):** A second, wider `<v-navigation-drawer>` will serve as the contextual panel. Its content will change dynamically based on the mode selected in the Application Switcher.
    *   **Main Content:** The central `<v-main>` area remains the primary workspace, containing the tabbed query editor.

2.  **Connection Management (`ConnectionManager.vue`)**
    *   **Location:** This new component will be the primary view within the "Active Panel" when the user is in "Connection Manager" mode. It replaces the concept of a simple modal dialog.
    *   **Functionality:** It will provide a rich interface for managing database connections. A `<v-list>` will display all saved connections. Selecting a connection will show its details in a form. Buttons will allow for creating, editing, and deleting connections. The form will use standard Vuetify components like `<v-text-field>` and `<v-select>`.

3.  **Database Explorer (`DbExplorer.vue`)**
    *   **Location:** This component will be displayed in the "Active Panel" when the user is in "Database Explorer" mode.
    *   **Component:** We will construct a tree using nested `<v-list>` and `<v-list-group>` components, as `v-treeview` is not yet available in Vuetify 3.
    *   **Interaction:** The tree will support lazy-loading of nodes. Right-clicking a node will trigger a `<v-menu>` for context-sensitive actions.

4.  **Query Workspace (`QueryTabs.vue` & `QueryView.vue`)**
    *   **This section remains unchanged from the previous plan.** It will consist of a `<v-tabs>` container managing multiple `QueryView` instances. Each `QueryView` will have a vertical split-pane layout for the Monaco editor and the `<v-data-table>` for results.

---

## Part 3: Technical Task List - Reusable Design Components

This phase focuses on creating a small library of generic, reusable components that form the building blocks of our application, ensuring consistency and accelerating development.

*   **Task 1: `AppLayout.vue`**
    *   **Description:** Implement the main application shell, configuring the new three-panel layout: the rail drawer for mode switching, the main content panel drawer, and the `<v-main>` component.
*   **Task 2: `AppTreeview.vue`**
    *   **Description:** Create a reusable, generic treeview component using Vuetify's `<v-list>` and `<v-list-group>`.
    *   **Props:** It will accept a `nodes` array as a prop and emit events for node expansion (`@expand`) and context-menu requests (`@contextmenu`).
*   **Task 3: `AppDataTable.vue`**
    *   **Description:** A standardized wrapper around `<v-data-table>`.
    *   **Props:** It will accept `columns` and `items` as props and will internally manage loading and empty states.
*   **Task 4: `AppSplitter.vue`**
    *   **Description:** A simple, reusable vertical splitter component using CSS and JavaScript to allow resizing between two panes.

---

## Part 4: Technical Task List - Application Implementation

This phase focuses on assembling the reusable components and implementing the application's features and business logic.

*   **Task 1: Project Setup & Vuetify Integration**
    *   `1.1:` Install `vuetify`, `@mdi/font`, and `vite-plugin-vuetify`.
    *   `1.2:` Create `frontend/src/plugins/vuetify.ts`.
    *   `1.3:` Update `frontend/main.ts` to use Vuetify and Pinia.
    *   `1.4:` Update `frontend/vite.config.ts`.

*   **Task 2: Implement Core Layout and State**
    *   `2.1:` Implement the new three-panel layout in `AppLayout.vue` (or directly in `App.vue`).
    *   `2.2:` Create the initial Pinia stores: `connection.ts`, `explorer.ts`, `query.ts`, and `tabs.ts`. Also create a new `ui.ts` store to manage the active panel mode.

*   **Task 3: Implement Connection Management**
    *   `3.1:` Build the `ConnectionManager.vue` component with a list for saved connections and a form for editing/creating them.
    *   `3.2:` Implement the logic in the `connection` store to save, delete, and retrieve connections from the backend.
    *   `3.3:` Wire the component to the store to provide full connection management functionality.

*   **Task 4: Implement the Database Explorer**
    *   `4.1:` Populate the `explorer` store with actions for lazy-loading tree data.
    *   `4.2:` Build `DbExplorer.vue`, which will use the `AppTreeview.vue` component.
    *   `4.3:` Place `DbExplorer.vue` inside the "Active Panel", to be shown when the UI is in "Database Explorer" mode.
    *   `4.4:` Implement context menu actions.

*   **Task 5: Implement the Query Workspace**
    *   `5.1:` Build `QueryTabs.vue` using `<v-tabs>`.
    *   `5.2:` Build `QueryView.vue` containing the Monaco editor and results area, separated by `AppSplitter.vue`.
    *   `5.3:` Use `AppDataTable.vue` to display query results.
    *   `5.4:` Implement the "Execute Query" logic in the `query` store.
