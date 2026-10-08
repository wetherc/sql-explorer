// The entry of the screenshot page. It puts the sample backend in place of
// the Tauri bridge and then starts the application as `src/main.ts` does.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { createBackend } from './backend'

mockWindows('main')
const backend = createBackend()
// Every command of the interface sends a record of named fields.
mockIPC((command, payload) => backend(command, payload as Record<string, unknown> | undefined), {
  shouldMockEvents: true,
})

await import('@/main')
