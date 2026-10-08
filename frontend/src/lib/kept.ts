import { api } from './api'

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
