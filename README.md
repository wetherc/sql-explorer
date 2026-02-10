# SQL Explorer

A lightweight, modern, and cross-platform desktop application for interacting with Microsoft SQL Server. Built with Vue.js and [Tauri](https://tauri.app/).

This is vibe coded slop. Because I want to be a better informed hater.

## Prerequisites

Before you begin, ensure you have the following installed:

1.  **Node.js and pnpm**
    -   Node.js version `^20.19.0` or `>=22.12.0`. We recommend using a version manager like [nvm](https://github.com/nvm-sh/nvm).
    -   `pnpm` is used for package management. Install it globally via npm:
        ```sh
        npm install -g pnpm
        ```
        or from [pnpm](https://pnpm.io/installation)

2.  **Rust**
    -   The backend is powered by Rust. Install the Rust toolchain via `rustup`.
    -   [Install Rust](https://www.rust-lang.org/tools/install)

### System Prerequisites (Ubuntu/Debian)

For building the Rust backend and its Tauri dependencies on Ubuntu/Debian-based systems, you will need to install several development libraries. Open your terminal and run the following command:

```sh
sudo apt update
sudo apt install -y pkg-config libssl-dev libsoup2.4-dev libjavascriptcoregtk-4.0-dev libgtk-3-dev libkrb5-dev libwebkit2gtk-4.0-dev libclang-dev
```

3.  **Tauri System Dependencies**
    -   Tauri requires system-level dependencies for building and webview rendering. Follow the official guide for your operating system:
    -   [Tauri Prerequisites Guide](https://tauri.app/v1/guides/getting-started/prerequisites)

## Project Setup

Clone the repository and install the frontend and backend dependencies:

```sh
pnpm install
```

This will also install the `@tauri-apps/cli` as a dev dependency, which is used to interface with the Tauri backend.

## Development

To run the application in development mode, which includes hot-reloading for the Vue frontend, use the Tauri CLI:

```sh
pnpm tauri dev
```

## Building for Production

To compile and build the final desktop application for your platform:

```sh
pnpm tauri build
```

The executable will be located in `src-tauri/target/release/`.

## Testing and Linting

-   **Run Unit Tests:**
    ```sh
    pnpm test:unit
    ```
-   **Lint and Format:**
    ```sh
    pnpm lint
    pnpm format
    ```

---

### Recommended IDE Setup

-   [VS Code](https://code.visualstudio.com/)
-   [Vue (Official)](https://marketplace.visualstudio.com/items?itemName=Vue.volar) (Volar)
-   [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) for Rust support.
