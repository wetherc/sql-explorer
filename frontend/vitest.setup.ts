import { vi } from 'vitest'

// Vuetify asks the host about the media it renders on.
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
})

// Vuetify positions a menu or a dialog against the visual viewport, which
// the test environment does not provide.
Object.defineProperty(window, 'visualViewport', {
  writable: true,
  value: {
    width: 1280,
    height: 800,
    offsetLeft: 0,
    offsetTop: 0,
    scale: 1,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  },
})

// An overlay of Vuetify looks for the element below the pointer to decide
// how the page scrolls behind it.
if (!document.elementsFromPoint) {
  document.elementsFromPoint = () => []
}

// The grid and the overlays of Vuetify measure the elements they sit on.
class StubResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
Object.defineProperty(window, 'ResizeObserver', {
  writable: true,
  value: StubResizeObserver,
})
Object.defineProperty(globalThis, 'ResizeObserver', {
  writable: true,
  value: StubResizeObserver,
})

// The editor is loaded only when the real application runs. The tests
// replace it, because the worker of the editor has no place in a test
// environment.
vi.mock('@/plugins/monaco', () => {
  const disposable = { dispose: vi.fn() }
  return {
    configureMonacoEnvironment: vi.fn(),
    registerMonacoThemes: vi.fn(),
    monaco: {
      editor: {
        create: vi.fn(() => ({
          getValue: vi.fn(() => ''),
          setValue: vi.fn(),
          getModel: vi.fn(() => null),
          getSelection: vi.fn(() => null),
          getPosition: vi.fn(() => null),
          onDidChangeModelContent: vi.fn(),
          addCommand: vi.fn(),
          addAction: vi.fn(() => disposable),
          updateOptions: vi.fn(),
          executeEdits: vi.fn(),
          focus: vi.fn(),
          dispose: vi.fn(),
        })),
        defineTheme: vi.fn(),
        setTheme: vi.fn(),
      },
      languages: {
        registerCompletionItemProvider: vi.fn(() => disposable),
        CompletionItemKind: {
          Keyword: 1,
          Module: 2,
          Folder: 3,
          Struct: 4,
          Field: 5,
        },
      },
      // The real values, so that two shortcuts never fold onto one key.
      KeyMod: { CtrlCmd: 2048, Shift: 1024, Alt: 512 },
      KeyCode: { Enter: 3, KeyF: 36 },
    },
  }
})
