import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type { ErrorPayload } from '@/types/api'
import { errorAdvice, errorIcon, fullErrorText, isCancellation, toErrorPayload } from '@/lib/errors'

export type NoticeLevel = 'success' | 'info' | 'warning' | 'error'

export interface Notice {
  id: number
  level: NoticeLevel
  message: string
  detail: string | null
  icon: string
  /** The number of milliseconds the notice stays on screen. */
  timeout: number
}

let nextNoticeId = 0

/**
 * The number of notices the corner holds. Each notice stands above the one
 * before it, so a burst of them would otherwise walk off the top of the
 * window. The oldest goes when a new one arrives above this number.
 */
export const MAX_NOTICES = 4

/** How long an error stays when the same failure is written somewhere else. */
export const ERROR_TIMEOUT_MS = 8000

/**
 * The notices the application shows, and the state of the panels the user
 * can open and close.
 */
export const useUiStore = defineStore('ui', () => {
  const notices = ref<Notice[]>([])
  /** The notice whose whole text the user asked to read. */
  const openedNotice = ref<Notice | null>(null)
  /** True while the command palette stands open. */
  const paletteOpen = ref(false)
  /** True while the list of the keys stands open. */
  const keyboardHelpOpen = ref(false)
  /**
   * The number of dialogs that stand open. Each dialog of the application
   * counts itself here as it opens and takes itself away as it closes, so the
   * key handler of the shell knows without asking the document.
   */
  const openDialogs = ref(0)
  const dialogOpen = computed(() => openDialogs.value > 0)

  function push(notice: Omit<Notice, 'id'>): Notice {
    const created: Notice = { ...notice, id: (nextNoticeId += 1) }
    notices.value.push(created)
    if (notices.value.length > MAX_NOTICES) {
      notices.value = notices.value.slice(notices.value.length - MAX_NOTICES)
    }
    return created
  }

  function dismiss(id: number): void {
    notices.value = notices.value.filter((notice) => notice.id !== id)
    if (openedNotice.value?.id === id) {
      openedNotice.value = null
    }
  }

  function clear(): void {
    notices.value = []
    openedNotice.value = null
  }

  function success(message: string): Notice {
    return push({
      level: 'success',
      message,
      detail: null,
      icon: 'mdi-check-circle-outline',
      timeout: 3000,
    })
  }

  function info(message: string): Notice {
    return push({
      level: 'info',
      message,
      detail: null,
      icon: 'mdi-information-outline',
      timeout: 4000,
    })
  }

  function warn(message: string, detail: string | null = null): Notice {
    return push({
      level: 'warning',
      message,
      detail,
      icon: 'mdi-alert-outline',
      timeout: 6000,
    })
  }

  /**
   * Reports a failed command. A command the user stopped raises no alarm, so
   * it becomes a short note instead.
   *
   * An error stays in the corner until the user takes it away, because a
   * failure that goes by itself can be missed. A caller that writes the same
   * failure somewhere the user can read it again, such as the messages of a
   * tab, marks it as `kept: true`. The notice is then a passing word about
   * something the user can still go back to, and it leaves on its own.
   */
  function reportError(error: unknown, options: { kept?: boolean } = {}): ErrorPayload {
    const payload = toErrorPayload(error)
    if (isCancellation(payload)) {
      info('The statement was stopped.')
      return payload
    }
    const advice = errorAdvice(payload)
    push({
      level: 'error',
      message: payload.message,
      detail: [advice, payload.detail].filter(Boolean).join('\n\n') || null,
      icon: errorIcon(payload.kind),
      timeout: options.kept ? ERROR_TIMEOUT_MS : -1,
    })
    return payload
  }

  function openNotice(notice: Notice): void {
    openedNotice.value = notice
  }

  function closeNotice(): void {
    openedNotice.value = null
  }

  /**
   * The two overlays of the shell live here, because the editor and the
   * key handler both open them and neither one is a child of the shell.
   */
  function setPaletteOpen(open: boolean): void {
    paletteOpen.value = open
  }

  function setKeyboardHelpOpen(open: boolean): void {
    keyboardHelpOpen.value = open
  }

  function addDialog(): void {
    openDialogs.value += 1
  }

  function removeDialog(): void {
    openDialogs.value = Math.max(0, openDialogs.value - 1)
  }

  return {
    notices,
    openedNotice,
    paletteOpen,
    keyboardHelpOpen,
    openDialogs,
    dialogOpen,
    addDialog,
    removeDialog,
    setPaletteOpen,
    setKeyboardHelpOpen,
    push,
    dismiss,
    clear,
    success,
    info,
    warn,
    reportError,
    openNotice,
    closeNotice,
    fullErrorText,
  }
})
