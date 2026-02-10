# Lessons Learned from Gemini CLI Session

This document outlines key lessons and actionable insights gained during interactions with the Gemini CLI, particularly when working on the `sql-explorer` project. These lessons are recorded to improve future efficiency and avoid common pitfalls.

## 1. Project-Specific Package Managers

**Issue:** Incorrectly using `npm` for dependency management in a project that uses `pnpm`.
**Impact:**
-   Installation failures (`npm ERR!`).
-   Corruption of dependency trees and `node_modules`.
-   Generation of unintended lockfiles (`package-lock.json` alongside `pnpm-lock.yaml`).
-   Wasted time debugging environment-related issues.
**Actionable Insight:**
-   **Always** check for the project's designated package manager at the outset of any frontend task. Look for lockfiles (`pnpm-lock.yaml`, `yarn.lock`, `package-lock.json`) or the `engines` field in `package.json`.
-   Strictly adhere to the detected package manager for all dependency-related operations (`install`, `add`, `remove`, `run`).

## 2. Tauri CLI Version Mismatch

**Issue:** Attempting to run a Tauri application with a CLI tool version (e.g., `cargo-tauri-cli` or `@tauri-apps/cli`) that is incompatible with the project's Tauri framework version.
**Impact:**
-   Configuration parsing errors (`tauri.conf.json` errors related to missing/unexpected properties).
-   Inability to launch the application in development mode.
-   Debugging efforts diverted to tooling compatibility instead of code.
**Actionable Insight:**
-   Verify the Tauri framework version specified in `Cargo.toml` (`tauri = { version = "X.Y" }`) and `package.json` (`@tauri-apps/api": "^X.Y"`).
-   Explicitly install a compatible version of the Tauri CLI. For `pnpm` (or `npm`), use `pnpm add -D @tauri-apps/cli@X.Y.Z`. For `cargo`, use `cargo install tauri-cli --version X.Y.Z`. Avoid relying on implicit "latest" installs if compatibility is unknown.

## 3. Rust Trait Object Bounds (`Send + Sync`) and Lifetime Errors (`E0277`, `E0308`)

**Issue:** Persistent and subtle type mismatch errors, particularly `E0277` (trait bound not satisfied) and `E0308` (incompatible types), when working with `Box<dyn Trait>` and `async_trait`, especially when `Send + Sync` bounds are involved. A recurring symptom was `Box<Box<...>>` in error messages.
**Impact:** Significant time spent on debugging Rust's type system and compiler inference.
**Actionable Insight:**
-   **Consistency is Key:** Ensure that `Send` and `Sync` bounds are consistently applied across:
    -   The trait definition itself (`pub trait DatabaseDriver: Send + Sync`).
    -   The trait object type in shared state (`Mutex<Option<Box<dyn DatabaseDriver + Send + Sync>>>`).
    -   The return types of functions that produce the boxed trait objects (e.g., `connect` methods of drivers returning `Result<Box<dyn DatabaseDriver + Send + Sync>, Error>`).
-   **Examine Intermediate Types:** When facing `Box<Box<...>>` errors, carefully inspect the return types of *all* intermediate expressions. A function might already be returning a `Box`, leading to an unintended double-box.
-   **Simplify to Isolate:** When stuck on type errors, simplify the code as much as possible to narrow down the problem. Remove explicit casts or type annotations temporarily to observe compiler inference.
-   **Idiomatic Patterns:** Prefer idiomatic patterns for boxing results, like having concrete driver `connect` methods return `Result<Box<dyn Trait>, Error>` directly, to allow the compiler to handle the coercion more smoothly.

## 4. Graceful Test Skipping for Integration Tests

**Issue:** Integration tests panicking or failing if required external resources (like a database connection via an environment variable) are not configured.
**Impact:** Leads to misleading test failures, preventing a full green build, and requiring manual context to understand why tests are failing.
**Actionable Insight:**
-   For integration tests that depend on environment-specific configurations (e.g., `MYSQL_TEST_DB_URL`), implement a mechanism to check for the required environment variables.
-   If the environment variable is missing, gracefully skip the test (e.g., by printing a message and `return`ing early in a `#[tokio::test]` function, or using `#[ignore]` with a conditional `#[cfg_attr]`). This allows the overall test suite to pass while still indicating that certain tests were not fully executed.

## 5. Commit Message Escaping in `run_shell_command`

**Issue:** Backticks (`` ` ``) within commit messages passed to `git commit -m '...'` via `run_shell_command` were being interpreted as shell command substitutions, causing the operation to be cancelled or resulting in syntax errors.
**Impact:** Leads to failed or malformed commits, requiring additional steps to amend.
**Actionable Insight:**
-   When using `run_shell_command` to execute `git commit -m`, ensure that any backticks within the commit message string are properly escaped (e.g., replace `` ` `` with ` ` `).
-   Alternatively, for complex messages or those prone to shell interpretation issues, consider using `git commit --amend --file=-` and piping the message content to avoid shell parsing of the message string itself.
