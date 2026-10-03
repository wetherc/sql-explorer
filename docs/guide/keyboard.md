---
title: Keyboard shortcuts
description: Keyboard shortcuts for the editor, the tabs, the panels, the object explorer and the results grid.
order: 9
---

# Keyboard shortcuts

Every command in the application comes from one registry, and the command
palette lists them all with their keys:

- `Ctrl`/`Cmd` + `Shift` + `P` opens the palette.
- `F1` opens the list of shortcuts.

The shortcut list names every command that has a key, so it stays up to date as
commands are added, and this guide names no key that the list doesn't already
show.

## Editor, tabs and panels

| Keys                             | Action                           |
| -------------------------------- | -------------------------------- |
| `Ctrl`/`Cmd` + `Enter`           | Run statement                    |
| `Ctrl`/`Cmd` + `Shift` + `Enter` | Run script                       |
| `Ctrl`/`Cmd` + `S`               | Save query                       |
| `Shift` + `Alt` + `F`            | Format SQL                       |
| `Ctrl`/`Cmd` + `T` or `N`        | New query                        |
| `Ctrl`/`Cmd` + `O`               | Open query…                      |
| `Ctrl`/`Cmd` + `Shift` + `O`     | Open folder…                     |
| `Ctrl`/`Cmd` + `W`               | Close tab                        |
| `Ctrl`/`Cmd` + `1` to `4`        | Show one of the four side panels |
| `Ctrl`/`Cmd` + `B`               | Toggle side panel                |
| `Ctrl`/`Cmd` + `J`               | Toggle results panel             |
| `Ctrl`/`Cmd` + `,`               | Open settings                    |

## Object explorer

The arrow keys move through the tree, with the right arrow expanding a branch
and the left arrow collapsing it. `Home` and `End` jump to the first and last
rows, and typing a letter jumps to the next row that starts with it.

## Results grid

The arrow keys move between cells, and `Page Up` and `Page Down` move a page of
rows at a time. `Home` and `End` go to the ends of a row, or to the ends of the
grid with `Ctrl`/`Cmd`. `Enter` opens the cell's whole value. `Space` selects
the row, and `Ctrl`/`Cmd` + `Space` adds the row to the selection.

`Ctrl`/`Cmd` + `A` selects every row that the filter shows, and `Ctrl`/`Cmd` +
`C` copies the selected rows, or the current cell's value when no row is
selected. Text that you highlight with the pointer copies as it is.

The menu key and `Shift` + `F10` open the cell's context menu, and when the menu
closes, the focus goes back to the cell.
