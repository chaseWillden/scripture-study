//! Scripture Study desktop app.
//!
//! Notes are stored as Markdown files in a directory; see [`library`].

mod app;
mod app_menu;
mod citation_form;
mod editor;
mod file_menu;
mod find_bar;
mod icons;
mod library;
mod link_menu;
mod marks;
mod menu;
mod meta;
mod reader;
mod scripture_menu;
mod shortcuts;
mod sidebar;
mod spell;
mod theme;

use eframe::egui;
use scripture_study_core::FsStore;

fn main() -> eframe::Result {
    let dir = library::dir();
    let store = FsStore::open(&dir)
        .unwrap_or_else(|e| panic!("can't open notes directory {}: {e}", dir.display()));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Scripture Study")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([480.0, 320.0])
            // macOS: draw under a transparent title bar, keeping the traffic lights.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "Scripture Study",
        options,
        Box::new(|cc| {
            app_menu::install(&cc.egui_ctx);
            Ok(Box::new(app::ScriptureStudyApp::new(cc, store)?))
        }),
    )
}
