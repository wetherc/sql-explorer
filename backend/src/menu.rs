//! The menu of the window, which the operating system draws.
//!
//! macOS holds one menu bar for the whole application, and Windows and Linux
//! draw the menu inside the window. Tauri builds one menu for all three, so
//! this module names the items once.
//!
//! The menu carries the standard items of the platform beside the items of
//! this application. Without the standard items a window of macOS loses the
//! Edit menu, and with it the keys that cut, copy and paste inside a text
//! field. The File menu therefore grows out of the one the platform gives,
//! and takes nothing away.
//!
//! Each item of this application carries the identifier of a command of the
//! interface. A click sends that identifier to the window, which runs the
//! command it names, so a command reached from the menu and a command
//! reached from a key follow one path.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

/// The event that carries the identifier of a command to the window.
pub const MENU_COMMAND_EVENT: &str = "menu-command";

/// One item of the menu that names a command of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuCommand {
    /// The identifier of the command in the interface.
    pub id: &'static str,
    pub label: &'static str,
    /// The key of the item, in the form that the platform reads. `CmdOrCtrl`
    /// stands for Command on macOS and for Control elsewhere.
    pub accelerator: &'static str,
}

/// The items of the File menu, in the order a reader expects them: the new
/// one, the two that open, and the one that writes.
pub const FILE_COMMANDS: [MenuCommand; 4] = [
    MenuCommand {
        id: "tab.new",
        label: "New Query",
        accelerator: "CmdOrCtrl+N",
    },
    MenuCommand {
        id: "file.open",
        label: "Open Query…",
        accelerator: "CmdOrCtrl+O",
    },
    MenuCommand {
        id: "file.openFolder",
        label: "Open Folder of Queries…",
        accelerator: "CmdOrCtrl+Shift+O",
    },
    MenuCommand {
        id: "query.save",
        label: "Save Query",
        accelerator: "CmdOrCtrl+S",
    },
];

/// Builds the menu of the application.
///
/// The File menu holds the items of this application, a line, and then the
/// item of the platform that closes the window. The other menus are the ones
/// the platform expects, so the keys that work in every application still
/// work in this one.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let mut file_items: Vec<MenuItem<R>> = Vec::new();
    for command in FILE_COMMANDS {
        file_items.push(MenuItem::with_id(
            app,
            command.id,
            command.label,
            true,
            Some(command.accelerator),
        )?);
    }
    let separator = PredefinedMenuItem::separator(app)?;
    let close = PredefinedMenuItem::close_window(app, None)?;

    let mut file_refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = Vec::new();
    for item in &file_items {
        file_refs.push(item);
    }
    file_refs.push(&separator);
    file_refs.push(&close);
    #[cfg(not(target_os = "macos"))]
    let quit = PredefinedMenuItem::quit(app, None)?;
    #[cfg(not(target_os = "macos"))]
    file_refs.push(&quit);

    let file_menu = Submenu::with_items(app, "File", true, &file_refs)?;

    // The Edit menu carries the keys of a text field. A window that holds a
    // menu of its own loses them when the menu does not name them.
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let app_menu = {
        let name = app.package_info().name.clone();
        Submenu::with_items(
            app,
            name,
            true,
            &[
                &PredefinedMenuItem::about(app, None, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?
    };

    #[cfg(target_os = "macos")]
    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[&PredefinedMenuItem::fullscreen(app, None)?],
    )?;

    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &app_menu,
            &file_menu,
            &edit_menu,
            #[cfg(target_os = "macos")]
            &view_menu,
            &window_menu,
        ],
    )
}

/// True when the identifier of a menu item names a command of the interface.
/// An item of the platform, such as the one that closes the window, carries
/// an identifier of its own and never reaches the interface.
pub fn names_a_command(id: &str) -> bool {
    FILE_COMMANDS.iter().any(|command| command.id == id)
}

/// Turns one item of the menu on or off.
///
/// The state of a command lives in the interface, which tells the backend
/// what changed. An item that the menu does not hold is left alone, so an
/// identifier of a command that carries no item costs nothing.
///
/// Returns true when the menu held the item.
pub fn set_command_enabled<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    enabled: bool,
) -> tauri::Result<bool> {
    let Some(menu) = app.menu() else {
        return Ok(false);
    };
    // The items of this application live one level down, in the File menu,
    // so the walk goes through the submenus.
    for kind in menu.items()? {
        let Some(submenu) = kind.as_submenu() else {
            continue;
        };
        let Some(found) = submenu.get(id) else {
            continue;
        };
        let Some(item) = found.as_menuitem() else {
            continue;
        };
        item.set_enabled(enabled)?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_item_of_the_file_menu_names_a_command_and_a_key() {
        let ids: Vec<&str> = FILE_COMMANDS.iter().map(|item| item.id).collect();
        assert_eq!(
            ids,
            vec!["tab.new", "file.open", "file.openFolder", "query.save"]
        );
        for item in FILE_COMMANDS {
            assert!(!item.label.is_empty());
            // Every key of the menu holds the modifier of the platform, so
            // one text serves macOS and the rest.
            assert!(item.accelerator.starts_with("CmdOrCtrl+"));
        }
    }

    #[test]
    fn an_item_of_the_platform_names_no_command() {
        assert!(names_a_command("tab.new"));
        assert!(names_a_command("query.save"));
        // The items that the platform builds carry their own identifiers.
        assert!(!names_a_command("close_window"));
        assert!(!names_a_command(""));
    }
}
