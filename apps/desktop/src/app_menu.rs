//! The macOS menu bar. Elsewhere there's none, and ⌘O is handled in-app.

/// A menu bar command that was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    OpenFolder,
    Settings,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Command;
    use std::sync::Mutex;

    use eframe::egui;
    use muda::{
        accelerator::{Accelerator, Code, Modifiers},
        Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    };

    const OPEN_FOLDER: &str = "open-folder";
    const SETTINGS: &str = "settings";

    /// Replaces the default menu bar with the app's.
    pub fn install(ctx: &egui::Context) {
        let open = MenuItem::with_id(
            OPEN_FOLDER,
            "Open Folder…",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
        );
        let settings = MenuItem::with_id(
            SETTINGS,
            "Settings…",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::Comma)),
        );
        let app = Submenu::with_items(
            "Scripture Study",
            true,
            &[
                &PredefinedMenuItem::about(None, None),
                &PredefinedMenuItem::separator(),
                &settings,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ],
        );
        let file = Submenu::with_items("File", true, &[&open]);
        let (Ok(app), Ok(file)) = (app, file) else {
            return;
        };
        let Ok(menu) = Menu::with_items(&[&app, &file]) else {
            return;
        };
        menu.init_for_nsapp();
        // The menu bar lives as long as the app.
        std::mem::forget(menu);
        // Menu clicks don't wake egui on their own.
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                OPEN_FOLDER => Some(Command::OpenFolder),
                SETTINGS => Some(Command::Settings),
                _ => None,
            };
            if let Some(command) = command {
                CHOSEN.lock().unwrap().push(command);
            }
            ctx.request_repaint();
        }));
    }

    /// Filled on the thread menu events arrive on, drained by the app.
    static CHOSEN: Mutex<Vec<Command>> = Mutex::new(Vec::new());

    /// Commands chosen since the last call.
    pub fn chosen() -> Vec<Command> {
        std::mem::take(&mut CHOSEN.lock().unwrap())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Command;
    use eframe::egui;

    pub fn install(_ctx: &egui::Context) {}

    pub fn chosen() -> Vec<Command> {
        Vec::new()
    }
}

pub use imp::{chosen, install};
