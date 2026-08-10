import { defineStore } from 'pinia'
import { ref } from 'vue'
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
 * The notices the application shows, and the state of the panels the user
 * can open and close.
 */
export const useUiStore = defineStore('ui', () => {
  const notices = ref<Notice[]>([])
  /** The notice whose whole text the user asked to read. */
  const openedNotice = ref<Notice | null>(null)

  function push(notice: Omit<Notice, 'id'>): Notice {
    const created: Notice = { ...notice, id: (nextNoticeId += 1) }
    notices.value.push(created)
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

  function warn(message: string): Notice {
    return push({
      level: 'warning',
      message,
      detail: null,
      icon: 'mdi-alert-outline',
      timeout: 6000,
    })
  }

  /**
   * Reports a failed command. A command the user stopped raises no alarm,
   * so it becomes a short note instead.
   */
  function reportError(error: unknown): ErrorPayload {
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
      // An error stays until the user removes it.
      timeout: -1,
    })
    return payload
  }

  function openNotice(notice: Notice): void {
    openedNotice.value = notice
  }

  function closeNotice(): void {
    openedNotice.value = null
  }

  return {
    notices,
    openedNotice,
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
