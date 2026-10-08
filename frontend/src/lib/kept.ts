import { api } from './api'
import type { Settings } from '@/stores/settings'
import type { SpillRequest } from '@/types/api'

/** The bytes of one gigabyte of the disk limit in Settings. */
export const GIGABYTE = 1024 ** 3

/**
 * What a normal run asks for when the user turned on saved full results, or
 * nothing when the option is off.
 */
export function spillRequest(
  settings: Pick<Settings, 'keepFullResults' | 'exportRowLimit' | 'fullResultsDiskGb'>,
): SpillRequest | undefined {
  return settings.keepFullResults
    ? { maxRows: settings.exportRowLimit, maxBytes: settings.fullResultsDiskGb * GIGABYTE }
    : undefined
}

/**
 * The seconds that a read of a normal run may pause at the row limit, or 0
 * for a read that ends there. Saved full results already give every row for
 * an export, so a run that saves them never pauses.
 */
export function pauseSeconds(
  settings: Pick<Settings, 'pauseAtRowLimit' | 'pauseLimitMinutes' | 'keepFullResults'>,
): number {
  return settings.pauseAtRowLimit && !settings.keepFullResults ? settings.pauseLimitMinutes * 60 : 0
}

/**
 * Lets the backend forget the kept result of each pane that leaves the
 * interface. The release is not awaited, and a release that fails needs no
 * notice, because the backend removes an old kept result on its own.
 */
export function releaseKept(panes: ReadonlyArray<{ keptId?: string }>): void {
  for (const pane of panes) {
    const keptId = pane.keptId
    if (keptId) {
      void Promise.resolve()
        .then(() => api.releaseKept(keptId))
        .catch(() => {})
    }
  }
}
