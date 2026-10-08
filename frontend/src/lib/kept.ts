import { api } from './api'
import { formatDuration, formatRowCount } from './format'
import type { Settings } from '@/stores/settings'
import type { KeptInfo, SpillRequest, UnsavedReason } from '@/types/api'

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

/** Writes a size with few digits, such as "412 MB" or "1.2 GB". */
export function shortSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const digits = unit === 0 || value >= 10 ? 0 : 1
  return `${value.toFixed(digits)} ${units[unit]}`
}

/** Writes a large count with few digits, such as "1.2M". */
export function shortCount(count: number): string {
  return new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 }).format(
    count,
  )
}

/** Writes how long ago a moment was, such as "3 h ago". */
export function timeAgo(at: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000)
  if (minutes < 1) {
    return 'just now'
  }
  return minutes < 60 ? `${minutes} min ago` : `${Math.floor(minutes / 60)} h ago`
}

/** Says where the full rows of a cut result are, for the grid. */
export function keptNote(kept: KeptInfo, now: number): string {
  switch (kept.origin) {
    case 'athena':
      return `Saved on Athena, ${timeAgo(kept.keptAt, now)}`
    case 'spill':
      return `Saved on this computer, ${formatRowCount(kept.savedRows ?? 0)}, ${shortSize(kept.savedBytes ?? 0)}`
    case 'paused':
      return `Paused on the server, ${timeAgo(kept.keptAt, now)}`
  }
}

/** Says why the full rows of a cut result were not saved, for the grid. */
export function unsavedNote(reason: UnsavedReason): string {
  switch (reason) {
    case 'script':
      return "Not saved: scripts with more than one statement can't be saved"
    case 'diskLimit':
      return 'Not saved: the disk limit was reached'
    case 'exportLimit':
      return 'Not saved: the export row limit was reached'
    case 'stopped':
      return 'Not saved: the read stopped'
    case 'diskFailed':
      return "Not saved: the file couldn't be written on this computer"
    case 'stoppedSaving':
      return 'Not saved: you stopped saving'
  }
}

/**
 * Says where an export of all rows takes its rows from and what that costs.
 * A result with no kept rows runs its query again, and the time of its last
 * run tells the user how long that takes.
 */
export function exportAllNote(kept: KeptInfo | null, runMs: number | null): string {
  switch (kept?.origin) {
    case 'athena':
      return "From the saved result (doesn't run again)"
    case 'spill':
      return 'From the rows saved on this computer'
    case 'paused':
      return 'Continues the paused read'
    default:
      return runMs === null
        ? 'Runs the query again'
        : `Runs the query again (last run took ${formatDuration(runMs)})`
  }
}
