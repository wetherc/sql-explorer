import { fileURLToPath } from 'node:url'
import { mergeConfig, defineConfig } from 'vite'
import { configDefaults } from 'vitest/config'
import viteConfig from './vite.config'

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      globals: true,
      environment: 'happy-dom',
      exclude: [...configDefaults.exclude, 'e2e/**'],
      root: fileURLToPath(new URL('./', import.meta.url)),
      server: {
        deps: {
          inline: ['vuetify'],
        },
      },
      setupFiles: ['./vitest.setup.ts'],
      coverage: {
        provider: 'v8',
        reporter: ['text-summary', 'json', 'html'],
        include: ['src/**/*.{ts,vue}'],
        exclude: [
          // Entry points and plugin wiring hold no branches of their own.
          'src/main.ts',
          'src/plugins/**',
          'src/env.d.ts',
          'src/**/*.d.ts',
          // The test helpers are part of the suite, not of the application.
          'src/**/__tests__/**',
        ],
        thresholds: {
          // Every module that carries logic is covered completely.
          'src/{lib,stores,types}/**/*.ts': {
            lines: 100,
            branches: 100,
            functions: 100,
            statements: 100,
          },
          // The views are covered completely as well, with one exception
          // that no test can remove. Vue compiles a `v-if` that has no
          // `v-else` into a cached comment node, and the coverage tool
          // never records that side as run, even when one test renders
          // the block and another leaves it out. The floor below is what
          // those cached nodes leave behind.
          lines: 100,
          functions: 100,
          statements: 100,
          branches: 96,
        },
      },
    },
  }),
)
