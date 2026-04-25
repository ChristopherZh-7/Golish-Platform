//! Native macOS menu construction extracted from `run_gui::setup`.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Runtime};

/// Build the native application menu (Golish / File / Edit / View / Window)
/// and install it on the given Tauri app. The caller should also wire up
/// [`install_menu_event_handler`] to forward menu clicks to the frontend.
pub(crate) fn install_app_menu<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let handle = app.handle();

    let app_menu = Submenu::with_items(
        handle,
        "Golish Platform",
        true,
        &[
            &PredefinedMenuItem::about(handle, Some("About Golish Platform"), None)?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(handle, "settings", "Settings...", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::services(handle, Some("Services"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, Some("Hide Golish Platform"))?,
            &PredefinedMenuItem::hide_others(handle, Some("Hide Others"))?,
            &PredefinedMenuItem::show_all(handle, Some("Show All"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, Some("Quit Golish Platform"))?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        handle,
        "File",
        true,
        &[
            &MenuItem::with_id(
                handle,
                "open-project",
                "Open Project...",
                true,
                Some("CmdOrCtrl+O"),
            )?,
            &MenuItem::with_id(
                handle,
                "new-project",
                "New Project...",
                true,
                Some("CmdOrCtrl+N"),
            )?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::close_window(handle, Some("Close Window"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        handle,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(handle, Some("Undo"))?,
            &PredefinedMenuItem::redo(handle, Some("Redo"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, Some("Cut"))?,
            &PredefinedMenuItem::copy(handle, Some("Copy"))?,
            &PredefinedMenuItem::paste(handle, Some("Paste"))?,
            &PredefinedMenuItem::select_all(handle, Some("Select All"))?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        handle,
        "View",
        true,
        &[&PredefinedMenuItem::fullscreen(
            handle,
            Some("Toggle Full Screen"),
        )?],
    )?;

    let window_menu = Submenu::with_items(
        handle,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(handle, Some("Minimize"))?,
            &PredefinedMenuItem::maximize(handle, Some("Zoom"))?,
        ],
    )?;

    let menu = Menu::with_items(
        handle,
        &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu],
    )?;
    app.set_menu(menu)?;

    app.on_menu_event(|app_handle, event| match event.id().as_ref() {
        "open-project" => {
            app_handle.emit("menu-open-project", ()).ok();
        }
        "new-project" => {
            app_handle.emit("menu-new-project", ()).ok();
        }
        "settings" => {
            app_handle.emit("menu-settings", ()).ok();
        }
        _ => {}
    });

    Ok(())
}
