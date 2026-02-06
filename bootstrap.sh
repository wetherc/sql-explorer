#!/bin/bash
#
# This script bootstraps the project by installing all dependencies.

# Exit immediately if a command exits with a non-zero status.
set -e

echo "--- Installing all dependencies via pnpm ---"
pnpm install

echo "--- Bootstrap complete! ---"
echo "You can now run 'pnpm dev' to start the application."
