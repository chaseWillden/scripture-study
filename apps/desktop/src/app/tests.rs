//! Headless UI tests that drive the real app with simulated input.
//!
//! Set `NOTES_SNAPSHOT_DIR` to also render PNG screenshots of each scenario.

use std::fs;
use std::thread;
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Key};
use egui_kittest::{kittest::Queryable, Harness};
use scripture_study_core::{scriptures, BlockKind};

use super::ScriptureStudyApp;

struct Fixture {
    dir: tempfile::TempDir,
    harness: Harness<'static, ScriptureStudyApp>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_notes(&[])
    }

    fn with_note(markdown: &str) -> Self {
        Self::with_notes(&[("note", markdown)])
    }

    /// Starts with these notes on disk; the last one is the most recent.
    fn with_notes(notes: &[(&str, &str)]) -> Self {
        Self::with_notes_sized(egui::vec2(1000.0, 640.0), notes)
    }

    fn with_notes_sized(size: egui::Vec2, notes: &[(&str, &str)]) -> Self {
        let mut fixture = Self::with_notes_sized_default(size, notes);
        fixture.harness.state_mut().sidebar.view = crate::sidebar::View::Recent;
        fixture.harness.run();
        fixture
    }

    fn with_notes_sized_default(size: egui::Vec2, notes: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let start = SystemTime::now() - Duration::from_secs(3 * 86_400);
        for (n, (id, markdown)) in notes.iter().enumerate() {
            let path = dir.path().join(format!("{id}.md"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, markdown).unwrap();
            let modified = start + Duration::from_secs(3600 * n as u64);
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(modified).unwrap();
        }
        let path = dir.path().to_path_buf();
        let mut harness = Harness::builder().with_size(size).build_eframe(move |cc| {
            ScriptureStudyApp::new(cc, scripture_study_core::FsStore::open(&path).unwrap()).unwrap()
        });
        harness.run();
        Self { dir, harness }
    }

    fn shortcut(&mut self, key: Key) {
        self.harness
            .key_press_modifiers(egui::Modifiers::COMMAND, key);
        self.harness.run();
    }

    /// Clicks an item in the open context menu. Menus draw on top, so the
    /// item is the last widget with that label.
    fn click_menu_item(&mut self, label: &str) {
        self.harness.get_all_by_label(label).last().unwrap().click();
        self.harness.run();
    }

    fn click(&mut self, label: &str) {
        self.harness.get_by_label(label).click();
        self.harness.run();
    }

    fn app(&self) -> &ScriptureStudyApp {
        self.harness.state()
    }

    fn type_text(&mut self, text: &str) {
        self.harness.event(egui::Event::Text(text.into()));
        self.harness.run();
    }

    fn press(&mut self, key: Key) {
        self.harness.key_press(key);
        self.harness.run();
    }

    fn wait_for_scripture_index(&mut self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while scriptures::search_if_ready("").is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "scripture index did not load"
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.harness.run();
    }

    fn wait_for_scripture_result(&mut self, label: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while self.harness.query_by_label(label).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "missing scripture result: {label}"
            );
            thread::sleep(Duration::from_millis(1));
            self.harness.run();
        }
    }

    fn shortcut_mods(&mut self, modifiers: egui::Modifiers, key: Key) {
        self.harness.key_press_modifiers(modifiers, key);
        self.harness.run();
    }

    fn kinds(&self) -> Vec<BlockKind> {
        self.app()
            .editor
            .doc
            .blocks
            .iter()
            .map(|b| b.kind.clone())
            .collect()
    }

    fn texts(&self) -> Vec<String> {
        self.app()
            .editor
            .doc
            .blocks
            .iter()
            .map(|b| b.text.clone())
            .collect()
    }

    fn saved_markdown(&mut self) -> String {
        self.wait_for_autosave();
        let id = self.app().current.clone();
        fs::read_to_string(self.dir.path().join(format!("{id}.md"))).unwrap()
    }

    /// Lets simulated time pass so the debounced autosave fires.
    fn wait_for_autosave(&mut self) {
        for _ in 0..60 {
            self.harness.step();
        }
        assert!(self.app().dirty_since.is_none(), "autosave didn't run");
    }

    fn snapshot(&mut self, name: &str) {
        let Some(dir) = std::env::var_os("NOTES_SNAPSHOT_DIR") else {
            return;
        };
        let image = self.harness.render().expect("render");
        image
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn default_sidebar_view_is_folders() {
    let f = Fixture::with_notes_sized_default(egui::vec2(1000.0, 640.0), &[]);
    assert_eq!(f.app().sidebar.view, crate::sidebar::View::Folders);
}

#[test]
fn starts_with_a_blank_focused_page() {
    let mut f = Fixture::new();
    assert_eq!(f.texts(), [""]);
    f.type_text("hello");
    assert_eq!(f.texts(), ["hello"]);
    f.snapshot("blank");
}

#[test]
fn slash_opens_menu_and_typing_filters_it() {
    let mut f = Fixture::new();
    f.type_text("/");
    assert!(f.harness.query_by_label("Heading 1").is_some());
    assert!(f.harness.query_by_label("Quote").is_some());
    f.snapshot("slash_menu");

    f.type_text("quo");
    assert!(f.harness.query_by_label("Quote").is_some());
    assert!(f.harness.query_by_label("Heading 1").is_none());
    f.snapshot("slash_menu_filtered");
}

#[test]
fn escape_closes_menu_and_keeps_text() {
    let mut f = Fixture::new();
    f.type_text("/x");
    f.type_text("");
    f.type_text("/");
    f.press(Key::Escape);
    assert!(f.harness.query_by_label("Heading 1").is_none());
    assert_eq!(f.texts(), ["/x/"]);
}

#[test]
fn choosing_a_command_converts_the_block() {
    let mut f = Fixture::new();
    f.type_text("/h1");
    f.press(Key::Enter);
    f.type_text("Groceries");
    assert_eq!(f.kinds(), [BlockKind::Heading(1)]);
    assert_eq!(f.texts(), ["Groceries"]);

    f.press(Key::Enter);
    f.type_text("/todo");
    f.press(Key::Enter);
    f.type_text("Milk");
    f.press(Key::Enter);
    f.type_text("Eggs");

    assert_eq!(
        f.saved_markdown(),
        "# Groceries\n\n- [ ] Milk\n- [ ] Eggs\n"
    );
}

#[test]
fn arrow_keys_pick_menu_items() {
    let mut f = Fixture::new();
    f.type_text("/head");
    f.press(Key::ArrowDown);
    f.press(Key::Enter);
    assert_eq!(f.kinds(), [BlockKind::Heading(2)]);
}

#[test]
fn clicking_a_menu_item_runs_it() {
    let mut f = Fixture::new();
    f.type_text("/");
    f.harness.get_by_label("Bulleted list").click();
    f.harness.run();
    f.type_text("item");
    assert_eq!(f.kinds(), [BlockKind::Bullet]);
    assert_eq!(f.texts(), ["item"]);
}

#[test]
fn star_or_dash_starts_a_bullet() {
    let mut f = Fixture::new();
    f.type_text("*");
    f.type_text(" ");
    assert_eq!(f.kinds(), [BlockKind::Bullet]);
    assert_eq!(f.texts(), [""]);
    f.type_text("milk");
    assert_eq!(f.texts(), ["milk"]);

    f.press(Key::Enter);
    f.press(Key::Enter); // empty item leaves the list
    f.type_text("-");
    f.type_text(" ");
    f.type_text("eggs");
    assert_eq!(f.kinds(), [BlockKind::Bullet, BlockKind::Bullet]);
    assert_eq!(f.texts(), ["milk", "eggs"]);

    // A marker at the start of a new line inside a paragraph.
    f.press(Key::Enter);
    f.press(Key::Enter);
    f.type_text("Keep");
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::Enter);
    f.harness.run();
    f.type_text("* ");
    f.type_text("item");
    assert_eq!(
        f.kinds(),
        [
            BlockKind::Bullet,
            BlockKind::Bullet,
            BlockKind::Paragraph,
            BlockKind::Bullet
        ]
    );
    assert_eq!(f.texts(), ["milk", "eggs", "Keep", "item"]);
}

#[test]
fn text_reflows_when_the_window_widens() {
    let body = format!("{}end", "brethren ".repeat(70));
    let markdown = format!("# Title\n\n{body}\n");
    let mut narrow = Fixture::with_notes_sized(egui::vec2(1000.0, 800.0), &[("note", &markdown)]);
    let mut wide = Fixture::with_notes_sized(egui::vec2(1700.0, 800.0), &[("note", &markdown)]);
    for fixture in [&mut narrow, &mut wide] {
        fixture
            .harness
            .ctx
            .all_styles_mut(|style| style.animation_time = 0.0);
        fixture.shortcut(Key::Backslash);
    }
    let rect = |fixture: &Fixture| fixture.harness.get_by_value(&body).rect();
    let narrow_r = rect(&narrow);
    let wide_r = rect(&wide);
    assert!(
        wide_r.height() < narrow_r.height(),
        "wider window should unwrap lines ({} vs {})",
        wide_r.height(),
        narrow_r.height()
    );
    // Rail is 52 wide and the sidebar is closed. The page keeps its margin
    // on both sides, past where the paragraph numbers hang.
    let left_inset = narrow_r.left() - 52.0;
    let right_inset = 1000.0 - narrow_r.right();
    assert!(
        left_inset >= 60.0 && right_inset >= 60.0,
        "page should sit in from both edges (left {left_inset}, right {right_inset})"
    );
}

#[test]
fn markdown_shortcuts_and_list_editing() {
    let mut f = Fixture::new();
    f.type_text("- ");
    assert_eq!(f.kinds(), [BlockKind::Bullet]);
    f.type_text("one");
    f.press(Key::Enter);
    f.type_text("two");
    f.press(Key::Enter);
    f.press(Key::Enter); // empty item leaves the list
    f.type_text("after");
    assert_eq!(
        f.kinds(),
        [BlockKind::Bullet, BlockKind::Bullet, BlockKind::Paragraph]
    );

    f.press(Key::Home);
    f.press(Key::Backspace); // merges into the item above
    assert_eq!(f.texts(), ["one", "twoafter"]);
}

#[test]
fn inline_markdown_is_kept_in_storage() {
    let mut f = Fixture::new();
    f.type_text("Some **bold** and *italic* and `code`");
    assert_eq!(
        f.saved_markdown(),
        "Some **bold** and *italic* and `code`\n"
    );
}

#[test]
fn renders_existing_markdown_as_rich_text() {
    let mut f = Fixture::with_note(
        "# Groceries\n\nThings to **buy** this *week*, see `list.txt`.\n\n\
         - Milk\n- Eggs\n\n1. First\n2. Second\n\n- [ ] Call mom\n- [x] Pay rent\n\n\
         > Be kind\n\n---\n\n```rust\nfn main() {}\n```\n",
    );
    assert_eq!(f.kinds().len(), 11);
    // Click away from the text so every block shows its rendered form.
    f.harness.state_mut().editor = crate::editor::Editor::new("other", f.app().editor.doc.clone());
    f.harness.run();
    f.snapshot("rich_text");
    f.harness.ctx.set_theme(egui::Theme::Light);
    f.harness.run();
    f.snapshot("rich_text_light");
}

#[test]
fn new_note_and_switching_notes() {
    let mut f = Fixture::new();
    f.type_text("First note");
    f.wait_for_autosave();
    let first = f.app().current.clone();

    f.type_text(" /new note");
    f.press(Key::Enter);
    assert_ne!(f.app().current, first);
    assert_eq!(f.texts(), [""]);
    assert_eq!(
        fs::read_to_string(f.dir.path().join(format!("{first}.md"))).unwrap(),
        "First note\n"
    );

    // The new note is empty, so switching away discards it.
    let second = f.app().current.clone();
    f.type_text("/first");
    f.press(Key::Enter);
    assert_eq!(f.app().current, first);
    assert!(!f.dir.path().join(format!("{second}.md")).exists());
}

#[test]
fn delete_note_removes_the_file() {
    let mut f = Fixture::new();
    f.type_text("Doomed");
    f.wait_for_autosave();
    let id = f.app().current.clone();
    f.type_text(" /delete");
    f.press(Key::Enter);
    // A fresh empty note replaces it (possibly reusing the timestamp id).
    assert!(f.app().editor.doc.is_blank());
    assert!(f.app().notes.iter().all(|n| n.title != "Doomed"));
    let path = f.dir.path().join(format!("{id}.md"));
    assert!(!path.exists() || fs::read_to_string(path).unwrap().is_empty());
}

#[test]
fn empty_blocks_size_the_caret_like_their_text() {
    use crate::theme;
    let mut harness = Harness::new_ui(|ui| {
        for kind in [
            BlockKind::Paragraph,
            BlockKind::Heading(1),
            BlockKind::Heading(2),
            BlockKind::Heading(3),
            BlockKind::Quote,
            BlockKind::Code {
                lang: String::new(),
            },
        ] {
            let empty = theme::layout(ui, "", &kind, false, true, 500.0);
            let full = theme::layout(ui, "Text", &kind, false, true, 500.0);
            assert_eq!(empty.rect.height(), full.rect.height(), "{kind:?}");
        }
    });
    harness.run();
}

#[test]
fn caret_snapshot() {
    let mut f = Fixture::new();
    f.harness.set_pixels_per_point(2.0);
    f.type_text("/h1");
    f.press(Key::Enter);
    f.snapshot("caret_empty_heading");
    f.type_text("Heading");
    f.snapshot("caret_heading");
}

fn three_notes() -> Fixture {
    Fixture::with_notes(&[
        ("groceries", "# Groceries\n\n- Milk\n- Eggs\n"),
        ("trip", "# Trip plan\n\nBook the hotel near the lake.\n"),
        ("ideas", "# Ideas\n\nA notes app with a slash menu.\n"),
    ])
}

#[test]
fn sidebar_lists_notes_and_opens_them() {
    let mut f = three_notes();
    assert_eq!(f.app().current, "ideas", "most recent note opens first");
    for title in ["Groceries", "Trip plan", "Ideas"] {
        assert!(f.harness.query_by_label(title).is_some(), "{title} missing");
    }
    f.snapshot("sidebar");
    f.harness.ctx.set_theme(egui::Theme::Light);
    f.harness.run();
    f.snapshot("sidebar_light");

    f.click("Trip plan");
    assert_eq!(f.app().current, "trip");
    assert_eq!(f.texts(), ["Trip plan", "Book the hotel near the lake."]);
}

#[test]
fn browsing_notes_keeps_their_order() {
    let mut f = three_notes();
    let order = |f: &Fixture| {
        f.app()
            .notes
            .iter()
            .map(|n| n.id.clone())
            .collect::<Vec<_>>()
    };
    let before = order(&f);
    f.click("Trip plan");
    f.click("Groceries");
    f.click("Ideas");
    assert_eq!(order(&f), before, "opening unchanged notes reordered them");

    // Editing still moves a note to the top once you leave it.
    f.click("Groceries");
    f.type_text("!");
    f.click("Trip plan");
    assert_eq!(f.app().notes[0].id, "groceries");
}

#[test]
fn search_filters_by_content_and_opens_with_enter() {
    let mut f = three_notes();
    f.shortcut(Key::K);
    f.type_text("lake");
    assert!(f.harness.query_by_label("Trip plan").is_some());
    assert!(f.harness.query_by_label("Groceries").is_none());
    assert!(f.harness.query_by_label("Ideas").is_none());
    f.snapshot("sidebar_search");

    f.press(Key::Enter);
    assert_eq!(f.app().current, "trip");
    assert!(!f.app().sidebar.is_searching());
    // Typing goes to the note again once search closes.
    f.type_text("!");
    assert_eq!(f.texts()[1], "Book the hotel near the lake.!");
}

#[test]
fn search_icon_opens_search_and_escape_closes_it() {
    let mut f = three_notes();
    f.click("Search notes");
    f.type_text("zzz");
    for title in ["Groceries", "Trip plan", "Ideas"] {
        assert!(f.harness.query_by_label(title).is_none());
    }
    f.press(Key::Escape);
    assert!(!f.app().sidebar.is_searching());
    assert!(f.harness.query_by_label("Groceries").is_some());
    // The note never received the search text.
    assert_eq!(f.texts(), ["Ideas", "A notes app with a slash menu."]);
}

#[test]
fn sidebar_toggles_with_button_and_shortcut() {
    let mut f = three_notes();
    // A long animation time so one frame only starts the slide.
    f.harness
        .ctx
        .all_styles_mut(|style| style.animation_time = 1.0);
    f.harness.get_by_label("Toggle sidebar").click();
    f.harness.step();
    assert!(!f.app().sidebar.open);
    assert!(
        f.harness.query_by_label("Groceries").is_some(),
        "the list stays through the slide"
    );
    f.harness.step();
    assert!(
        f.app().sidebar.panel_visible(),
        "closing eases out instead of vanishing"
    );
    f.harness
        .ctx
        .all_styles_mut(|style| style.animation_time = 0.0);
    f.harness.run();
    assert!(f.harness.query_by_label("Groceries").is_none());
    f.snapshot("sidebar_hidden");
    f.shortcut(Key::Backslash);
    assert!(f.app().sidebar.open);
    let before = f.texts();
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::B);
    assert!(!f.app().sidebar.open);
    // ⌘⇧B toggles the sidebar and does not also bold the focused text.
    assert_eq!(f.texts(), before);
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::B);
    assert!(f.app().sidebar.open);
}

#[test]
fn new_note_from_sidebar_rail_and_shortcut() {
    let mut f = three_notes();
    // One "New note" in the sidebar list, one in the rail. Both work.
    for n in 0..2 {
        let before = f.app().current.clone();
        f.harness
            .get_all_by_label("New note")
            .nth(n)
            .unwrap()
            .click();
        f.harness.run();
        assert_ne!(f.app().current, before, "button {n}");
        f.type_text("Fresh");
        assert_eq!(f.texts(), ["Fresh"], "typing lands in the new note ({n})");
        f.wait_for_autosave();
    }
    let before = f.app().current.clone();
    f.shortcut(Key::N);
    assert_ne!(f.app().current, before);
    assert!(f.app().editor.doc.is_blank());
}

#[test]
fn collapsed_sidebar_keeps_the_icon_rail() {
    let mut f = three_notes();
    f.click("Hide notes");
    assert!(!f.app().sidebar.open);
    assert!(f.harness.query_by_label("Groceries").is_none());
    for label in ["Show notes", "Scriptures", "Search notes", "New note"] {
        assert!(f.harness.query_by_label(label).is_some(), "{label} missing");
    }
    f.snapshot("rail_collapsed");

    // Search from the rail opens the sidebar straight into search.
    f.click("Search notes");
    assert!(f.app().sidebar.open && f.app().sidebar.is_searching());
    f.type_text("groc");
    assert!(f.harness.query_by_label("Groceries").is_some());

    // The notes button leaves search and shows the list.
    f.click("Show notes");
    assert!(f.app().sidebar.open && !f.app().sidebar.is_searching());
    assert!(f.harness.query_by_label("Trip plan").is_some());
}

#[test]
fn formatting_shortcuts_style_the_word_under_the_caret() {
    let mut f = Fixture::new();
    f.type_text("hello");
    f.shortcut(Key::B);
    assert_eq!(f.texts(), ["**hello**"]);
    f.shortcut(Key::B);
    assert_eq!(f.texts(), ["hello"]);

    f.shortcut(Key::I);
    assert_eq!(f.texts(), ["*hello*"]);
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::X);
    assert_eq!(f.texts(), ["*~~hello~~*"]);
}

#[test]
fn block_shortcuts_change_the_current_block() {
    let mut f = Fixture::new();
    f.type_text("Title");
    let alt = egui::Modifiers::COMMAND | egui::Modifiers::ALT;
    let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    f.shortcut_mods(alt, Key::Num1);
    assert_eq!(f.kinds(), [BlockKind::Heading(1)]);
    f.shortcut_mods(alt, Key::Num2);
    assert_eq!(f.kinds(), [BlockKind::Heading(2)]);
    f.shortcut_mods(shift, Key::Num8);
    assert_eq!(f.kinds(), [BlockKind::Bullet]);
    assert_eq!(f.texts(), ["Title"]);
    f.shortcut_mods(shift, Key::Num9);
    assert_eq!(f.kinds(), [BlockKind::Todo { checked: false }]);
    f.shortcut(Key::Enter);
    assert_eq!(f.kinds(), [BlockKind::Todo { checked: true }]);
    f.shortcut_mods(alt, Key::Num0);
    assert_eq!(f.kinds(), [BlockKind::Paragraph]);
}

#[test]
fn inline_shortcuts_leave_code_blocks_alone() {
    let mut f = Fixture::new();
    f.type_text("```");
    assert_eq!(
        f.kinds(),
        [BlockKind::Code {
            lang: String::new()
        }]
    );
    f.type_text("code");
    f.shortcut(Key::B);
    assert_eq!(f.texts(), ["code"]);
}

#[test]
fn find_shortcut_opens_the_find_bar_not_note_search() {
    let mut f = three_notes();
    f.shortcut(Key::F);
    assert!(f.app().editor.find.open);
    assert!(
        !f.app().sidebar.is_searching(),
        "Cmd+K is for searching all notes"
    );
    f.press(Key::Escape);
    assert!(!f.app().editor.find.open);
}

#[test]
fn save_and_delete_shortcuts() {
    let mut f = three_notes();
    // One frame, so the half-second autosave has not fired yet.
    f.harness.event(egui::Event::Text("!".into()));
    f.harness.step();
    assert!(f.app().dirty_since.is_some());
    f.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, Key::S);
    f.harness.step();
    assert!(f.app().dirty_since.is_none());
    assert_eq!(
        fs::read_to_string(f.dir.path().join("ideas.md")).unwrap(),
        "# Ideas\n\nA notes app with a slash menu.!\n"
    );

    f.shortcut_mods(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        Key::Backspace,
    );
    assert_ne!(f.app().current, "ideas");
    assert!(!f.dir.path().join("ideas.md").exists());
}

#[test]
fn sidebar_titles_follow_edits_live() {
    let mut f = three_notes();
    f.press(Key::ArrowUp); // caret starts at the end; move into the title
                           // Step frame by frame so the debounced autosave can't run yet.
    f.harness.event(egui::Event::Text(" v2".into()));
    f.harness.step();
    f.harness.step();
    assert_eq!(f.texts()[0], "Ideas v2");
    assert!(f.app().dirty_since.is_some(), "not saved yet");
    assert!(f.harness.query_by_label("Ideas v2").is_some());
}

#[test]
fn empty_note_shows_a_hint_even_when_unfocused() {
    use crate::editor::EMPTY_NOTE_HINT;
    let has_hint = |f: &Fixture| {
        f.harness
            .query_by(|n| n.placeholder() == Some(EMPTY_NOTE_HINT))
            .is_some()
    };

    let mut f = Fixture::new();
    assert!(has_hint(&f));

    // Move focus away from the page (as when clicking in the sidebar).
    f.shortcut(Key::K);
    assert!(f.app().sidebar.is_searching());
    assert!(has_hint(&f), "hint should stay while the page is unfocused");
    f.snapshot("empty_note_unfocused");

    f.press(Key::Escape);
    f.shortcut(Key::N); // opens a fresh note with the caret in it
    f.type_text("Hello");
    assert!(!has_hint(&f));
}

fn organized_notes() -> Fixture {
    let mut f = Fixture::with_notes(&[
        ("Work/plan", "# Plan\n\nShip it.\n"),
        ("groceries", "# Groceries\n\n- Milk\n"),
        ("ideas", "# Ideas\n"),
    ]);
    f.click("Organize notes");
    f
}

fn note_file(f: &Fixture, id: &str) -> std::path::PathBuf {
    f.dir.path().join(format!("{id}.md"))
}

#[test]
fn organize_view_shows_folders_that_expand() {
    let mut f = organized_notes();
    assert_eq!(f.app().sidebar.view, crate::sidebar::View::Folders);
    assert!(f.harness.query_by_label("Work").is_some());
    assert!(
        f.harness.query_by_label("Groceries").is_some(),
        "top-level notes"
    );
    assert!(f.harness.query_by_label("Plan").is_none(), "collapsed");

    f.click("Work");
    assert!(f.harness.query_by_label("Plan").is_some());
    f.snapshot("organize");

    f.click("Plan");
    assert_eq!(f.app().current, "Work/plan");

    // The rail button toggles the view closed again.
    f.click("Organize notes");
    assert!(!f.app().sidebar.open);
}

#[test]
fn folder_view_displays_numbered_notes_in_numeric_then_alphabetical_order() {
    let mut f = Fixture::with_notes_sized_default(
        egui::vec2(1000.0, 640.0),
        &[
            ("Prophecies/ten", "# 10 - Laman\n"),
            ("Prophecies/two-z", "# 2 - Zebra\n"),
            ("Prophecies/one", "# 1 - Destruction\n"),
            ("Prophecies/two-a", "# 2 - Apple\n"),
            ("Prophecies/summary", "# 1 Nephi Prophecies\n"),
            ("Prophecies/plain", "# Apple\n"),
        ],
    );
    f.click("Prophecies");
    let titles = [
        "1 - Destruction",
        "1 Nephi Prophecies",
        "2 - Apple",
        "2 - Zebra",
        "10 - Laman",
        "Apple",
    ];
    let positions: Vec<_> = titles
        .iter()
        .map(|title| f.harness.get_by_label(title).rect().top())
        .collect();
    assert!(
        positions.windows(2).all(|rows| rows[0] < rows[1]),
        "notes should appear in numeric then alphabetical order: {positions:?}"
    );
    f.click("2 - Apple");
    assert_eq!(f.app().current, "Prophecies/two-a");
}

#[test]
fn folder_tree_keyboard_navigation_opens_a_note() {
    let mut f = organized_notes();
    f.click("Work");

    // Work is selected by the click; the next visible row is its note.
    f.press(Key::ArrowDown);
    f.press(Key::ArrowUp);

    // Left collapses the selected folder; right opens it again and moves to
    // its first child when pressed a second time.
    f.press(Key::ArrowLeft);
    assert!(f.harness.query_by_label("Plan").is_none());
    f.press(Key::ArrowRight);
    f.press(Key::ArrowRight);
    // The headless harness applies row focus on the following frame.
    f.harness
        .state_mut()
        .sidebar
        .request_tree_focus("note:Work/plan".into());
    f.press(Key::Enter);
    assert_eq!(f.app().current, "Work/plan");
}

#[test]
fn document_enter_does_not_toggle_the_selected_folder() {
    let mut f = organized_notes();
    f.click("Work");

    // Move keyboard focus to the document while the folder remains selected.
    f.harness.get_all_by_value("Ideas").next().unwrap().click();
    f.press(Key::Enter);

    assert!(f.harness.query_by_label("Plan").is_some());
}

#[test]
fn document_cursor_does_not_toggle_folders() {
    let mut f = organized_notes();
    f.click("Work");
    f.click("Plan");

    f.shortcut_mods(egui::Modifiers::COMMAND, Key::ArrowLeft);
    f.shortcut_mods(egui::Modifiers::COMMAND, Key::ArrowRight);
    assert!(f.harness.query_by_label("Plan").is_some());
}

#[test]
fn create_a_folder_from_the_header_button() {
    let mut f = organized_notes();
    f.click("New folder");
    f.type_text("Projects");
    f.press(Key::Enter);
    assert!(f.dir.path().join("Projects").is_dir());
    assert!(f.harness.query_by_label("Projects").is_some());
}

#[test]
fn blank_folder_area_opens_creation_menu() {
    let mut f = organized_notes();
    let row = f.harness.get_by_label("Work").rect();
    let pos = egui::pos2(row.left() + 20.0, row.bottom() + 100.0);
    f.harness.hover_at(pos);
    f.harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    f.harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    f.harness.run();

    assert!(f.harness.get_all_by_label("New folder").count() >= 2);
    assert!(f.harness.query_by_label("New document").is_some());
}

#[test]
fn invalid_or_empty_folder_names_are_not_created() {
    let mut f = organized_notes();
    f.click("New folder");
    f.type_text(".hidden");
    f.press(Key::Enter);
    assert!(!f.dir.path().join(".hidden").exists());
    f.press(Key::Escape);
    assert!(f.app().sidebar_editing_is_none());
}

#[test]
fn folder_menu_creates_notes_and_subfolders_inside() {
    let mut f = organized_notes();
    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click_menu_item("New note");
    assert!(f.app().current.starts_with("Work/"));
    f.type_text("Inside");
    f.wait_for_autosave();
    assert_eq!(
        fs::read_to_string(note_file(&f, &f.app().current.clone())).unwrap(),
        "Inside\n"
    );

    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click_menu_item("New folder");
    f.type_text("Sub");
    f.press(Key::Enter);
    assert!(f.dir.path().join("Work/Sub").is_dir());
}

#[test]
fn renaming_a_folder_keeps_the_open_note() {
    let mut f = organized_notes();
    f.click("Work");
    f.click("Plan");
    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click("Rename");
    f.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    f.type_text("Jobs");
    f.press(Key::Enter);

    assert!(f.dir.path().join("Jobs/plan.md").exists());
    assert!(!f.dir.path().join("Work").exists());
    assert_eq!(f.app().current, "Jobs/plan");
    // Edits keep saving to the renamed location.
    f.harness.state_mut().editor.doc.blocks[1].text.push('!');
    f.harness.state_mut().save();
    assert!(fs::read_to_string(note_file(&f, "Jobs/plan"))
        .unwrap()
        .contains("Ship it.!"));
}

#[test]
fn folders_with_notes_cannot_be_deleted() {
    let mut f = organized_notes();
    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click("Delete folder");
    assert!(note_file(&f, "Work/plan").exists());

    // An empty folder can.
    fs::create_dir(f.dir.path().join("Empty")).unwrap();
    f.harness.state_mut().refresh_notes();
    f.harness.run();
    f.harness.get_by_label("Empty").click_secondary();
    f.harness.run();
    f.click("Delete folder");
    assert!(!f.dir.path().join("Empty").exists());
}

#[test]
fn drag_a_note_onto_a_folder() {
    let mut f = organized_notes();
    let from = f.harness.get_by_label("Groceries").rect().center();
    let to = f.harness.get_by_label("Work").rect().center();

    f.harness.hover_at(from);
    f.harness.run();
    f.harness.drag_at(from);
    f.harness.run();
    for step in 1..=5 {
        f.harness.hover_at(from.lerp(to, step as f32 / 5.0));
        f.harness.run();
    }
    f.harness.drop_at(to);
    f.harness.run();

    assert!(note_file(&f, "Work/groceries").exists());
    assert!(!note_file(&f, "groceries").exists());
    assert!(
        f.harness.query_by_label("Groceries").is_some(),
        "folder opens to show it"
    );
}

#[test]
fn move_the_open_note_from_its_menu() {
    let mut f = three_notes();
    fs::create_dir(f.dir.path().join("Archive")).unwrap();
    f.harness.state_mut().refresh_notes();
    f.harness.run();

    f.harness.get_by_label("Ideas").click_secondary();
    f.harness.run();
    f.harness.get_by_label("Move to").hover();
    f.harness.run();
    f.snapshot("move_submenu");
    f.click("Archive");

    assert_eq!(f.app().current, "Archive/ideas");
    assert!(note_file(&f, "Archive/ideas").exists());
    assert_eq!(f.texts(), ["Ideas", "A notes app with a slash menu."]);
}

#[test]
fn properties_roundtrip_tags_mentions_and_custom_fields() {
    let mut f = Fixture::new();
    assert!(f.harness.query_by_label_contains("Created").is_some());
    assert!(f.harness.query_by_label_contains("Updated").is_some());

    f.click("Add tags");
    assert!(
        f.harness.query_by_label("Tags").is_some(),
        "tag field opens"
    );
    f.type_text("design, ideas");
    f.press(Key::Enter);
    f.press(Key::Escape);

    f.click("Add people");
    f.type_text("@alex");
    f.press(Key::Enter);
    f.press(Key::Escape);

    // "Add property" shows while the row is hovered.
    let row = f.harness.get_by_label("Tag design").rect().center();
    f.harness.hover_at(row);
    f.harness.run();
    f.click("Add property");
    f.type_text("Status");
    f.press(Key::Enter);
    // The caret moves straight to the new property's value.
    f.type_text("draft");

    let doc = scripture_study_core::Document::from_markdown(&f.saved_markdown());
    assert_eq!(doc.properties.tags, ["design", "ideas"]);
    assert_eq!(doc.properties.mentions, ["alex"]);
    assert_eq!(
        doc.properties.extra,
        vec![("Status".into(), "draft".into())]
    );
    assert!(doc.properties.created.is_some());
    // The writing area is still there, under the properties.
    assert_eq!(f.texts(), [""]);
    f.snapshot("properties");

    // Hovering a chip reveals its remove button.
    assert!(f.harness.query_by_label("Remove tag design").is_none());
    f.harness
        .hover_at(f.harness.get_by_label("Tag design").rect().center());
    f.harness.run();
    f.click("Remove tag design");
    let doc = scripture_study_core::Document::from_markdown(&f.saved_markdown());
    assert_eq!(doc.properties.tags, ["ideas"]);
}

#[test]
fn enter_adds_a_tag_and_keeps_typing_in_the_field() {
    let mut f = Fixture::new();
    f.click("Add tags");
    for tag in ["alpha", "beta", "gamma"] {
        f.type_text(tag);
        f.press(Key::Enter);
    }
    let tags = f.app().editor.doc.properties.tags.clone();
    assert_eq!(tags, ["alpha", "beta", "gamma"]);
    for tag in ["alpha", "beta", "gamma"] {
        assert!(f.harness.query_by_label(&format!("Tag {tag}")).is_some());
    }
    // Nothing leaked into the note itself, and it still invites writing.
    assert_eq!(f.texts(), [""]);
    assert!(f
        .harness
        .query_by(|n| n.placeholder() == Some(crate::editor::EMPTY_NOTE_HINT))
        .is_some());
    f.snapshot("properties_compact");
    f.harness.ctx.set_theme(egui::Theme::Light);
    f.harness.run();
    f.snapshot("properties_light");
}

#[test]
fn enter_adds_mentions_one_after_another() {
    let mut f = Fixture::new();
    f.click("Add people");
    f.type_text("ana");
    f.press(Key::Enter);
    f.type_text("@ben");
    f.press(Key::Enter);
    assert_eq!(f.app().editor.doc.properties.mentions, ["ana", "ben"]);
}

#[test]
fn backspace_in_an_empty_tag_field_removes_the_last_tag() {
    let mut f = Fixture::new();
    f.click("Add tags");
    f.type_text("one, two");
    f.press(Key::Enter);
    f.press(Key::Backspace);
    assert_eq!(f.app().editor.doc.properties.tags, ["one"]);
}

// --- Selections that span blocks -------------------------------------------

const THREE_BLOCKS: &str = "# Title\n\nfirst line\n\nsecond line\n";

/// Screen rect of the block whose text is `text`.
fn block_rect(f: &Fixture, text: &str) -> egui::Rect {
    f.harness
        .get_by(|n| {
            n.value().as_deref() == Some(text)
                && n.role() == egui::accesskit::Role::MultilineTextInput
        })
        .rect()
}

fn selection(f: &Fixture) -> Option<scripture_study_core::selection::Selection> {
    f.app().editor.selection()
}

/// Sends a clipboard event and returns what was copied that frame.
fn clipboard_after(f: &mut Fixture, event: egui::Event) -> Option<String> {
    f.harness.event(event);
    f.harness.step();
    let copied = f
        .harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        });
    f.harness.run();
    copied
}

#[test]
fn cmd_a_selects_the_whole_document() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    f.shortcut(Key::A);
    let doc = f.app().editor.doc.clone();
    assert_eq!(
        selection(&f),
        Some(scripture_study_core::selection::Selection::all(&doc))
    );
    f.snapshot("select_all");

    assert_eq!(
        clipboard_after(&mut f, egui::Event::Copy).as_deref(),
        Some("# Title\n\nfirst line\n\nsecond line")
    );
    f.press(Key::Backspace);
    assert!(f.app().editor.doc.is_blank());
    assert_eq!(selection(&f), None);
    // Typing continues in the emptied note.
    f.type_text("fresh");
    assert_eq!(f.texts(), ["fresh"]);
}

#[test]
fn shift_up_extends_the_selection_row_by_row_across_blocks() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    // The caret starts at the end of "second line".
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowUp);
    f.harness.run();
    let sel = selection(&f).expect("selection into the block above");
    assert_eq!(
        sel.anchor,
        scripture_study_core::editor::Caret { block: 2, char: 11 }
    );
    assert_eq!(sel.head.block, 1);

    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowUp);
    f.harness.run();
    assert_eq!(selection(&f).unwrap().head.block, 0);
    f.snapshot("shift_up_selection");

    // Shift+Down shrinks it back.
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowDown);
    f.harness.run();
    assert_eq!(selection(&f).unwrap().head.block, 1);

    // Typing replaces everything selected.
    f.type_text("X");
    assert_eq!(f.texts().len(), 2);
    assert!(f.texts()[1].ends_with('X'), "{:?}", f.texts());
    assert_eq!(selection(&f), None);
}

#[test]
fn shift_arrows_inside_a_block_still_select_normally() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowLeft);
    f.harness.run();
    assert_eq!(
        selection(&f),
        None,
        "within one block the text field selects"
    );
    f.type_text("E");
    assert_eq!(f.texts()[2], "second linE");
}

#[test]
fn shift_left_at_the_start_of_a_block_crosses_over() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    f.press(Key::Home);
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowLeft);
    f.harness.run();
    let sel = selection(&f).unwrap();
    assert_eq!(
        sel.head,
        scripture_study_core::editor::Caret { block: 1, char: 10 }
    );

    // Plain arrows collapse the selection and put the caret back.
    f.press(Key::ArrowRight);
    assert_eq!(selection(&f), None);
    f.type_text("!");
    assert_eq!(f.texts()[2], "!second line");
}

#[test]
fn mouse_drag_selects_across_blocks() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    let from = block_rect(&f, "first line").left_center() + egui::vec2(2.0, 0.0);
    let to = block_rect(&f, "second line").center();

    f.harness.hover_at(from);
    f.harness.run();
    f.harness.drag_at(from);
    f.harness.run();
    for step in 1..=4 {
        f.harness.hover_at(from.lerp(to, step as f32 / 4.0));
        f.harness.run();
    }
    f.harness.drop_at(to);
    f.harness.run();

    let (start, end) = selection(&f).expect("drag selection").range();
    assert_eq!((start.block, start.char), (1, 0));
    assert_eq!(end.block, 2);
    f.snapshot("drag_selection");

    let copied = clipboard_after(&mut f, egui::Event::Cut).unwrap();
    assert!(copied.starts_with("first line\n\nsec"), "{copied:?}");
    assert_eq!(f.texts()[0], "Title");
    assert_eq!(f.texts().len(), 2);
}

#[test]
fn drag_inside_one_block_is_a_normal_text_selection() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    let rect = block_rect(&f, "first line");
    let (from, to) = (rect.left_center() + egui::vec2(1.0, 0.0), rect.center());
    f.harness.hover_at(from);
    f.harness.run();
    f.harness.drag_at(from);
    f.harness.run();
    f.harness.hover_at(to);
    f.harness.run();
    f.harness.drop_at(to);
    f.harness.run();
    assert_eq!(selection(&f), None);
    f.type_text("Z");
    assert!(f.texts()[1].starts_with('Z') && f.texts()[1].len() < "first line".len());
}

#[test]
fn pasting_a_list_into_a_list_item_keeps_the_rest() {
    let mut f = Fixture::with_note("1. Alpha\n2. Beta\n3. Gamma\n");
    // The caret starts at the end of Gamma. Move to the end of Beta, in the
    // middle of the list, and paste another list there.
    f.press(Key::ArrowUp);
    f.harness
        .event(egui::Event::Paste("1. One\n\t1. Nested\n2. Two".into()));
    f.harness.run();
    assert_eq!(
        f.texts(),
        ["Alpha", "Beta", "One", "Nested", "Two", "Gamma"]
    );
    let indents: Vec<u8> = f.app().editor.doc.blocks.iter().map(|b| b.indent).collect();
    assert_eq!(indents, [0, 0, 0, 1, 0, 0]);
    assert_eq!(
        f.app().editor.doc.list_number(4),
        4,
        "Two continues the list"
    );
    assert_eq!(f.app().editor.doc.list_number(3), 1, "Nested starts at 1");
    assert_eq!(
        f.app().editor.doc.list_number(5),
        5,
        "Gamma keeps its place"
    );
}

#[test]
fn pasting_over_a_selection_inserts_blocks() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    f.shortcut(Key::A);
    f.harness
        .event(egui::Event::Paste("# New\n\n- one\n- two".into()));
    f.harness.run();
    assert_eq!(f.texts(), ["New", "one", "two"]);
    assert_eq!(f.kinds()[1], BlockKind::Bullet);
}

#[test]
fn escape_and_clicks_clear_the_selection() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    f.shortcut(Key::A);
    f.press(Key::Escape);
    assert_eq!(selection(&f), None);

    f.shortcut(Key::A);
    let point = block_rect(&f, "first line").center();
    f.harness.hover_at(point);
    f.harness.run();
    f.harness.drag_at(point);
    f.harness.run();
    f.harness.drop_at(point);
    f.harness.run();
    assert_eq!(selection(&f), None);
    f.type_text("#");
    assert!(
        f.texts()[1].contains('#'),
        "click puts the caret in the block"
    );
}

/// Two real pointer clicks (accessibility clicks don't count as a double).
fn double_click(f: &mut Fixture, label: &str) {
    // The harness steps 0.25s per frame, slower than a real double-click.
    f.harness
        .ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    let pos = f.harness.get_by_label(label).rect().center();
    f.harness.hover_at(pos);
    f.harness.run();
    for pressed in [true, false, true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    f.harness.run();
}

#[test]
fn double_clicking_a_note_renames_it() {
    let mut f = three_notes();
    double_click(&mut f, "Groceries");
    assert_eq!(f.app().sidebar.renaming_note(), Some("Groceries"));
    // The old title starts selected, so typing replaces it.
    f.type_text("Shopping");
    f.press(Key::Enter);

    assert!(f.app().sidebar.renaming_note().is_none());
    assert_eq!(
        fs::read_to_string(note_file(&f, "groceries")).unwrap(),
        "# Shopping\n\n- Milk\n- Eggs\n"
    );
    assert!(f.harness.query_by_label("Shopping").is_some());
}

#[test]
fn renaming_a_note_keeps_focus_for_cursor_navigation() {
    let mut f = three_notes();
    f.shortcut(Key::R);

    // The title starts selected. Move to the end, then use a word-navigation
    // shortcut; both must be handled by the text field instead of ending the
    // rename.
    f.press(Key::ArrowRight);
    f.shortcut_mods(egui::Modifiers::ALT, Key::ArrowLeft);
    f.type_text("Big ");
    f.press(Key::Enter);

    assert!(f.app().sidebar.renaming_note().is_none());
    assert!(fs::read_to_string(note_file(&f, "ideas"))
        .unwrap()
        .starts_with("# Big Ideas\n"));
}

#[test]
fn rename_from_the_menu_and_escape_cancels() {
    let mut f = three_notes();
    f.harness.get_by_label("Trip plan").click_secondary();
    f.harness.run();
    f.snapshot("note_menu");
    f.click_menu_item("Rename");
    f.type_text("Vacation");
    f.press(Key::Escape);
    assert!(f.app().sidebar.renaming_note().is_none());
    assert!(fs::read_to_string(note_file(&f, "trip"))
        .unwrap()
        .starts_with("# Trip plan\n"));

    f.harness.get_by_label("Trip plan").click_secondary();
    f.harness.run();
    f.click_menu_item("Rename");
    f.type_text("Vacation");
    f.press(Key::Enter);
    assert!(fs::read_to_string(note_file(&f, "trip"))
        .unwrap()
        .starts_with("# Vacation\n"));
    assert_eq!(f.app().current, "ideas", "renaming doesn't open the note");
}

#[test]
fn rename_shortcut_renames_the_open_note() {
    let mut f = three_notes();
    f.shortcut(Key::R);
    assert_eq!(f.app().sidebar.renaming_note(), Some("Ideas"));
    f.type_text("Big ideas");
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["Big ideas", "A notes app with a slash menu."]);
    assert!(fs::read_to_string(note_file(&f, "ideas"))
        .unwrap()
        .starts_with("# Big ideas\n"));
}

#[test]
fn menu_shortcuts_act_on_the_menus_note() {
    let mut f = three_notes();
    f.harness.get_by_label("Groceries").click_secondary();
    f.harness.run();
    f.shortcut_mods(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        Key::Backspace,
    );
    assert!(!note_file(&f, "groceries").exists());
    assert!(note_file(&f, "ideas").exists(), "not the open note");
    assert_eq!(f.app().current, "ideas");
}

// --- Word and line selection (Option/Cmd + Shift + arrows) ------------------

const OPT_SHIFT: egui::Modifiers = egui::Modifiers {
    alt: true,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: false,
};
const CMD_SHIFT: egui::Modifiers = egui::Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: true,
    command: true,
};

fn chord(f: &mut Fixture, modifiers: egui::Modifiers, key: Key) {
    f.harness.key_press_modifiers(modifiers, key);
    f.harness.run();
}

fn caret(block: usize, char: usize) -> scripture_study_core::editor::Caret {
    scripture_study_core::editor::Caret { block, char }
}

#[test]
fn option_shift_selects_words_and_continues_into_the_block_above() {
    let mut f = Fixture::with_note(THREE_BLOCKS); // caret at end of "second line"
    chord(&mut f, OPT_SHIFT, Key::ArrowLeft); // "line"
    chord(&mut f, OPT_SHIFT, Key::ArrowLeft); // "second line"
    assert_eq!(selection(&f), None, "still inside one block");
    chord(&mut f, OPT_SHIFT, Key::ArrowLeft); // into "first line": "line"
    let sel = selection(&f).expect("crossed into the block above");
    assert_eq!((sel.anchor, sel.head), (caret(2, 11), caret(1, 6)));
    chord(&mut f, OPT_SHIFT, Key::ArrowLeft); // a word, not a character
    assert_eq!(selection(&f).unwrap().head, caret(1, 0));
    chord(&mut f, OPT_SHIFT, Key::ArrowRight);
    assert_eq!(selection(&f).unwrap().head, caret(1, 5));
}

#[test]
fn option_shift_inside_a_block_selects_the_word() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    chord(&mut f, OPT_SHIFT, Key::ArrowLeft);
    f.type_text("X");
    assert_eq!(f.texts()[2], "second X");
}

#[test]
fn cmd_shift_left_and_right_select_to_the_line_edges() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    chord(&mut f, CMD_SHIFT, Key::ArrowLeft);
    f.type_text("Y");
    assert_eq!(f.texts()[2], "Y", "selected back to the line start");

    // With a selection spanning blocks, the moving end goes to its line's edges.
    let mut f = Fixture::with_note(THREE_BLOCKS);
    chord(&mut f, egui::Modifiers::SHIFT, Key::ArrowUp);
    assert_eq!(selection(&f).unwrap().head.block, 1);
    chord(&mut f, CMD_SHIFT, Key::ArrowLeft);
    assert_eq!(selection(&f).unwrap().head, caret(1, 0));
    chord(&mut f, CMD_SHIFT, Key::ArrowRight);
    assert_eq!(selection(&f).unwrap().head, caret(1, 10));
}

#[test]
fn cmd_shift_up_and_down_select_to_the_ends_of_the_note() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    chord(&mut f, CMD_SHIFT, Key::ArrowUp);
    let sel = selection(&f).expect("selection to the top");
    assert_eq!((sel.anchor, sel.head), (caret(2, 11), caret(0, 0)));
    chord(&mut f, CMD_SHIFT, Key::ArrowDown);
    assert_eq!(selection(&f).unwrap().head, caret(2, 11));
}

#[test]
fn modified_arrows_without_shift_collapse_the_selection() {
    let mut f = Fixture::with_note(THREE_BLOCKS);
    chord(&mut f, CMD_SHIFT, Key::ArrowUp);
    chord(
        &mut f,
        egui::Modifiers {
            shift: false,
            ..CMD_SHIFT
        },
        Key::ArrowLeft,
    );
    assert_eq!(selection(&f), None);
    f.type_text(">");
    assert_eq!(f.texts()[0], ">Title");
}

fn tiny_png() -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 60, 60, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    png.into_inner()
}

/// What egui-winit sends for Cmd+V with only a picture on the clipboard:
/// nothing for the press, just the release of V.
fn paste_image(f: &mut Fixture) {
    f.harness.state_mut().clipboard_image = || Some(tiny_png());
    f.harness.event(egui::Event::Key {
        key: Key::V,
        physical_key: Some(Key::V),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    f.harness.run();
}

fn image_srcs(f: &Fixture) -> Vec<String> {
    f.app()
        .editor
        .doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Image { src, .. } => Some(src.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn pasting_a_screenshot_adds_an_image() {
    let mut f = Fixture::new();
    f.type_text("Look at this");
    paste_image(&mut f);

    let srcs = image_srcs(&f);
    assert_eq!(srcs.len(), 1);
    assert!(srcs[0].starts_with(".assets/") && srcs[0].ends_with(".png"));
    assert_eq!(fs::read(f.dir.path().join(&srcs[0])).unwrap(), tiny_png());
    // The caret waits in a fresh paragraph below.
    f.type_text("After");
    assert_eq!(f.texts(), ["Look at this", "", "After"]);
    assert_eq!(
        f.saved_markdown(),
        format!("Look at this\n\n![]({})\n\nAfter\n", srcs[0])
    );
    f.snapshot("pasted_image");

    // Pasting text still pastes text.
    f.harness.state_mut().clipboard_image = || panic!("text paste read the clipboard image");
    f.harness.event(egui::Event::Paste(" and more".into()));
    f.harness.event(egui::Event::Key {
        key: Key::V,
        physical_key: Some(Key::V),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    f.harness.run();
    assert_eq!(f.texts()[2], "After and more");
}

#[test]
fn a_pasted_image_replaces_an_empty_line_and_can_be_deleted() {
    let mut f = Fixture::new();
    f.type_text("Top");
    f.press(Key::Enter);
    paste_image(&mut f);
    assert_eq!(f.kinds().len(), 3, "the empty line became the image");
    assert!(matches!(f.kinds()[1], BlockKind::Image { .. }));

    f.click("Image");
    assert_eq!(f.app().editor.selected_image(), Some(1));
    f.press(Key::Backspace);
    assert_eq!(image_srcs(&f), Vec::<String>::new());
    assert_eq!(f.texts(), ["Top", ""]);
}

#[test]
fn dropping_a_picture_file_adds_it() {
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    let mut f = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("photo.PNG");
    fs::write(&path, tiny_png()).unwrap();
    f.harness
        .input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(path)));
    f.harness.run();
    let srcs = image_srcs(&f);
    assert_eq!(srcs.len(), 1);
    assert!(srcs[0].ends_with(".png"));
}

#[test]
fn pasting_a_link_cleans_it_up() {
    let mut f = Fixture::new();
    f.type_text("Read ");
    f.harness.event(egui::Event::Paste(
        "https://www.example.com/guides/rust/?utm_source=newsletter&id=4".into(),
    ));
    f.harness.run();
    assert_eq!(
        f.texts(),
        ["Read [example.com/guides/rust](https://www.example.com/guides/rust/?id=4)"]
    );

    // Pasting over selected text links that text.
    f.type_text(" and ");
    f.type_text("docs");
    for _ in 0..4 {
        f.shortcut_mods(egui::Modifiers::SHIFT, Key::ArrowLeft);
    }
    f.harness
        .event(egui::Event::Paste("https://docs.rs/egui?fbclid=xyz".into()));
    f.harness.run();
    assert!(f.texts()[0].ends_with(" and [docs](https://docs.rs/egui)"));
}

#[test]
fn clicking_a_link_opens_it_when_not_editing() {
    let mut f = Fixture::with_note("# Links\n\nSee [the docs](https://docs.rs) now\n");
    // Edit the heading, so the paragraph with the link isn't being edited.
    let heading = block_rect(&f, "Links").center();
    f.harness.hover_at(heading);
    f.harness.run();
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos: heading,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.run();
    }
    f.harness.run();

    let text = block_rect(&f, "See [the docs](https://docs.rs) now");
    // "See " is about 30pt wide at 16pt; aim inside "the docs".
    let pos = text.left_center() + egui::vec2(50.0, 0.0);
    f.harness.hover_at(pos);
    f.harness.run();
    let mut opened = None;
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.step();
        for command in &f.harness.output().platform_output.commands {
            if let egui::OutputCommand::OpenUrl(open) = command {
                opened = Some(open.url.clone());
            }
        }
    }
    assert_eq!(opened.as_deref(), Some("https://docs.rs"));
    f.harness.run();
    f.snapshot("link");
    assert!(
        f.harness
            .query_by(|n| n.is_focused()
                && n.value().as_deref() == Some("See [the docs](https://docs.rs) now"))
            .is_none(),
        "opening the link didn't start editing the paragraph"
    );
}

#[test]
fn links_open_on_click_even_while_editing_and_stay_clean() {
    // Blocks below the link must not swallow its click.
    let mut f = Fixture::with_note("Go to [the docs](https://docs.rs) please\n\nMore\n\nText\n");
    f.press(Key::ArrowUp);
    f.press(Key::ArrowUp);
    let text = "Go to [the docs](https://docs.rs) please";
    // The note opens with the caret in this paragraph, so it's being edited.
    assert!(f
        .harness
        .query_by(|n| n.is_focused() && n.value().as_deref() == Some(text))
        .is_some());
    f.snapshot("link_while_editing");

    let pos = block_rect(&f, text).left_center() + egui::vec2(70.0, 0.0);
    f.harness.hover_at(pos);
    f.harness.run();
    let mut opened = None;
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.step();
        for command in &f.harness.output().platform_output.commands {
            if let egui::OutputCommand::OpenUrl(open) = command {
                opened = Some(open.url.clone());
            }
        }
    }
    assert_eq!(opened.as_deref(), Some("https://docs.rs"));

    // Backspace right after the link edits its visible text, not the markup.
    f.press(Key::End);
    for _ in 0.." please".len() {
        f.press(Key::Backspace);
    }
    f.press(Key::Backspace);
    assert_eq!(f.texts()[0], "Go to [the doc](https://docs.rs)");
}

static REVEALED: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

#[test]
fn reveal_notes_and_folders_in_the_file_manager() {
    let mut f = organized_notes();
    f.harness.state_mut().reveal = |path| {
        REVEALED.lock().unwrap().push(path.to_path_buf());
        Ok(())
    };
    let label = crate::sidebar::REVEAL_LABEL;

    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click_menu_item(label);
    f.harness.get_by_label("Groceries").click_secondary();
    f.harness.run();
    f.click_menu_item(label);
    // ⌥⌘R reveals the open note.
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::ALT, Key::R);
    assert!(f.app().sidebar.renaming_note().is_none(), "not taken as ⌘R");

    let current = f.app().current.clone();
    let revealed: Vec<_> = REVEALED.lock().unwrap().drain(..).collect();
    assert_eq!(
        revealed,
        [
            f.dir.path().join("Work"),
            note_file(&f, "groceries"),
            note_file(&f, &current),
        ]
    );
}

static PICKED: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

#[test]
fn move_notes_and_folders_to_a_location_on_disk() {
    let mut f = organized_notes();
    let elsewhere = tempfile::tempdir().unwrap();
    *PICKED.lock().unwrap() = Some(elsewhere.path().to_path_buf());
    f.harness.state_mut().pick_folder = |_, _| PICKED.lock().unwrap().clone();
    let label = crate::sidebar::MOVE_OUT_LABEL;

    f.harness.get_by_label("Groceries").click_secondary();
    f.harness.run();
    f.click_menu_item(label);
    assert!(elsewhere.path().join("groceries.md").exists());
    assert!(!note_file(&f, "groceries").exists());
    assert!(f.harness.query_by_label("Groceries").is_none());

    f.harness.get_by_label("Work").click_secondary();
    f.harness.run();
    f.click_menu_item(label);
    assert!(elsewhere.path().join("Work/plan.md").exists());
    assert!(!f.dir.path().join("Work").exists());
    assert!(f.harness.query_by_label("Work").is_none());
    assert!(f.app().error.is_none());
}

static OPENED: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
static REMEMBERED: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

#[test]
fn open_another_folder_of_notes() {
    let mut f = three_notes();
    let other = tempfile::tempdir().unwrap();
    fs::create_dir(other.path().join("Study")).unwrap();
    fs::write(other.path().join("Study/nephi.md"), "# 1 Nephi\n").unwrap();
    let app = f.harness.state_mut();
    app.pick_folder = |_, _| OPENED.lock().unwrap().clone();
    app.remember_library = |dir| {
        REMEMBERED.lock().unwrap().push(dir.to_path_buf());
        Ok(())
    };
    *OPENED.lock().unwrap() = Some(other.path().to_path_buf());

    f.harness.state_mut().choose_library();
    f.harness.run();
    assert_eq!(f.app().current, "Study/nephi");
    assert_eq!(f.texts(), ["1 Nephi"]);
    assert!(f.harness.query_by_label("Ideas").is_none());
    assert_eq!(*REMEMBERED.lock().unwrap(), [other.path().to_path_buf()]);

    // Edits land in the new folder.
    f.type_text("!");
    f.wait_for_autosave();
    assert_eq!(
        fs::read_to_string(other.path().join("Study/nephi.md")).unwrap(),
        "# 1 Nephi!\n"
    );
}

#[test]
fn recent_notes_show_their_folders_under_the_title() {
    let mut f = Fixture::with_notes(&[
        ("Scripture Study/Hebrew/isaiah", "# Isaiah\n"),
        ("loose", "# Loose\n"),
        ("Scripture Study/Prophecies/nephi", "# 1 Nephi\n"),
    ]);
    f.snapshot("recent_folders");
    let height = |f: &Fixture, label: &str| f.harness.get_by_label(label).rect().height();
    assert!(
        height(&f, "Isaiah") > height(&f, "Loose"),
        "second line for the folder"
    );
    assert_eq!(height(&f, "1 Nephi"), height(&f, "Isaiah"));
}

// --- Caret size and find in note ------------------------------------------

#[test]
fn caret_is_as_tall_as_the_text_not_the_line() {
    use crate::{editor::caret_rect, theme};
    let mut harness = Harness::new_ui(|ui| {
        for kind in [
            BlockKind::Paragraph,
            BlockKind::Heading(1),
            BlockKind::Quote,
        ] {
            let text = "Two lines, wrapped: brethren shall be inasmuch as the teacher";
            let galley = theme::layout(ui, text, &kind, false, true, 180.0);
            assert!(galley.rows.len() > 1, "wraps");
            let line = galley.rows[1].height();
            let text_height = theme::text_height(ui, &kind);
            assert!(text_height < line, "{kind:?}: line spacing exists");
            let cursor = egui::text::CCursor::new(text.len() - 3);
            let caret = caret_rect(&galley, cursor, text_height);
            assert!(caret.height() <= text_height + 2.0, "{kind:?}: {caret:?}");
            // It starts at its own line, not the one above.
            let row_top = galley.pos_from_cursor(cursor).min.y;
            assert!((caret.min.y - row_top).abs() <= 1.0);
        }
    });
    harness.run();
}

const BRETHREN: &str =
    "# Brethren\n\nThe brethren shall be **inasmuch** as the teacher over them.\n";

/// Whether the find bar shows exactly this count, e.g. "1 of 2".
fn shows_count(f: &Fixture, count: &str) -> bool {
    f.harness.query_by_label(count).is_some()
}

#[test]
fn cmd_f_finds_in_the_note_and_steps_through_matches() {
    let mut f = Fixture::with_note(BRETHREN);
    f.shortcut(Key::F);
    f.type_text("brethren");
    assert_eq!(f.app().editor.find.matches.len(), 2);
    assert!(shows_count(&f, "1 of 2"), "expected 1 of 2");
    f.snapshot("find_bar");

    f.press(Key::Enter);
    assert!(shows_count(&f, "2 of 2"), "expected 2 of 2");
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::Enter);
    f.harness.run();
    assert!(shows_count(&f, "1 of 2"), "expected 1 of 2");
    f.click("Next match");
    assert_eq!(f.app().editor.find.current, 1);

    // Esc closes and leaves the match selected, so typing replaces it.
    f.press(Key::Escape);
    assert!(!f.app().editor.find.open);
    f.type_text("sisters");
    assert_eq!(
        f.texts()[1],
        "The sisters shall be **inasmuch** as the teacher over them."
    );
}

#[test]
fn find_matches_text_inside_formatting() {
    let mut f = Fixture::with_note(BRETHREN);
    f.shortcut(Key::F);
    f.type_text("be inasmuch");
    assert!(shows_count(&f, "1 of 1"), "expected 1 of 1");
}

#[test]
fn fuzzy_toggle_finds_near_misses() {
    let mut f = Fixture::with_note(BRETHREN);
    f.shortcut(Key::F);
    f.type_text("inasmch");
    assert!(shows_count(&f, "No results"), "expected No results");

    f.click("Fuzzy");
    assert!(f.app().editor.find.fuzzy);
    assert!(shows_count(&f, "1 of 1"), "expected 1 of 1");
    f.snapshot("find_bar_fuzzy");

    // The field keeps focus after toggling, so typing continues the query.
    f.type_text("x");
    assert_eq!(f.app().editor.find.query, "inasmchx");
}

#[test]
fn cmd_f_again_selects_the_query_to_replace_it() {
    let mut f = Fixture::with_note(BRETHREN);
    f.shortcut(Key::F);
    f.type_text("teacher");
    f.shortcut(Key::F);
    f.type_text("them");
    assert_eq!(f.app().editor.find.query, "them");
    assert!(shows_count(&f, "1 of 1"), "expected 1 of 1");
}

#[test]
fn tabbing_a_paragraph_restarts_its_number() {
    let mut f = Fixture::new();
    f.type_text("...that");
    f.press(Key::Enter);
    f.type_text("sdf");
    f.press(Key::Enter);
    f.type_text("...that sword.");
    assert_eq!(
        f.app().editor.doc.paragraph_numbers(),
        [Some(1), Some(2), Some(3)]
    );

    // Tab the middle paragraph. It starts again at 1, and the paragraph
    // under it keeps the outer count.
    f.press(Key::ArrowUp);
    f.press(Key::Tab);
    assert_eq!(f.texts()[1], "\tsdf");
    assert_eq!(
        f.app().editor.doc.paragraph_numbers(),
        [Some(1), Some(1), Some(2)]
    );
}

#[test]
fn collapsing_an_outline_item_hides_its_sub_items() {
    let mut f =
        Fixture::with_note("Parent\n\n\tChild one\n\n\tChild two\n\n\t\tGrandchild\n\nSibling\n");
    assert_eq!(
        f.texts(),
        [
            "Parent",
            "\tChild one",
            "\tChild two",
            "\t\tGrandchild",
            "Sibling"
        ]
    );
    // Only items that have something nested under them can fold.
    assert!(f.harness.query_by_label("Collapse outline 0").is_some());
    assert!(f.harness.query_by_label("Collapse outline 1").is_none());
    assert!(f.harness.query_by_label("Collapse outline 2").is_some());

    // The note opens with the caret in the last block. Walk up to the parent
    // before folding, so the click doesn't have to find the caret.
    for _ in 0..4 {
        f.press(Key::ArrowUp);
    }
    f.click("Collapse outline 0");
    assert!(f.harness.query_by_label("Expand outline 0").is_some());
    assert!(
        f.harness.query_by_label("Collapse outline 2").is_none(),
        "sub items are hidden"
    );
    assert_eq!(
        f.texts(),
        [
            "Parent",
            "\tChild one",
            "\tChild two",
            "\t\tGrandchild",
            "Sibling"
        ],
        "folding doesn't delete anything"
    );
    f.snapshot("outline_collapsed");

    // Arrow down skips the hidden sub items.
    f.press(Key::ArrowDown);
    f.type_text("X");
    assert_eq!(f.texts()[4], "XSibling");
    assert_eq!(f.texts()[1], "\tChild one");

    f.click("Expand outline 0");
    assert!(f.harness.query_by_label("Collapse outline 2").is_some());

    // Folding a nested item hides only its own sub items.
    f.click("Collapse outline 2");
    assert!(f.harness.query_by_label("Expand outline 2").is_some());
    assert!(f.harness.query_by_label("Collapse outline 0").is_some());
    f.snapshot("outline_nested_collapsed");
}

#[test]
fn collapsed_outline_is_remembered_in_the_notes_folder() {
    let mut f = Fixture::with_notes(&[
        ("other", "Other\n"),
        (
            "outline",
            "Parent\n\n\tChild\n\n\t\tGrandchild\n\nSibling\n",
        ),
    ]);
    assert_eq!(f.app().current, "outline");
    f.click("Collapse outline 0");

    let settings = fs::read_to_string(f.dir.path().join(".scripture-study")).unwrap();
    assert!(
        settings.contains("\"version\": 1"),
        "settings file:\n{settings}"
    );
    assert!(settings.contains("p:Parent"), "settings file:\n{settings}");
    // The other note in this folder has no entry of its own yet.
    assert!(!settings.contains("other"));

    f.click("Other");
    assert_eq!(f.app().current, "other");
    f.click("Parent");
    assert_eq!(f.app().current, "outline");
    assert!(
        f.harness.query_by_label("Expand outline 0").is_some(),
        "the parent stays folded"
    );
    assert!(
        f.harness.query_by_label("Collapse outline 1").is_none(),
        "its sub items stay hidden"
    );

    f.click("Expand outline 0");
    assert!(
        !f.dir.path().join(".scripture-study").exists(),
        "an unfolded note leaves no settings file"
    );
}

#[test]
fn paragraph_numbers_snapshot() {
    let mut f = Fixture::with_note(
        "# Isaiah 53\n\nWho hath believed our report?\n\nFor he shall grow up before him as a tender plant, and as a root out of a dry ground: he hath no form nor comeliness; and when we shall see him, there is no beauty that we should desire him.\n\n- a list item\n\nHe is despised and rejected of men.\n",
    );
    f.snapshot("paragraph_numbers");
    f.harness.ctx.set_theme(egui::Theme::Light);
    f.harness.run();
    f.snapshot("paragraph_numbers_light");
}

/// Types one character per frame, like a person typing steadily.
fn type_steadily(f: &mut Fixture, text: &str) {
    for c in text.chars() {
        f.harness.event(egui::Event::Text(c.to_string()));
        f.harness.step();
    }
    f.harness.run();
}

fn undo(f: &mut Fixture) {
    f.shortcut(Key::Z);
}

fn redo(f: &mut Fixture) {
    f.shortcut(Key::Y);
}

#[test]
fn undo_and_redo_typing_a_word_at_a_time() {
    let mut f = Fixture::new();
    type_steadily(&mut f, "hello world");
    assert_eq!(f.texts(), ["hello world"]);
    undo(&mut f);
    assert_eq!(f.texts(), ["hello "]);
    undo(&mut f);
    assert_eq!(f.texts(), [""]);
    redo(&mut f);
    assert_eq!(f.texts(), ["hello "]);
    // ⇧⌘Z redoes too.
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z);
    assert_eq!(f.texts(), ["hello world"]);
    // The caret is back in place: typing continues at the end.
    f.type_text("!");
    assert_eq!(f.texts(), ["hello world!"]);
    assert!(f.saved_markdown().starts_with("hello world!"));
}

#[test]
fn undo_bold_and_unbold() {
    let mut f = Fixture::new();
    type_steadily(&mut f, "word");
    f.shortcut(Key::B);
    assert_eq!(f.texts(), ["**word**"]);
    f.shortcut(Key::B);
    assert_eq!(f.texts(), ["word"], "unbolded");

    undo(&mut f);
    assert_eq!(f.texts(), ["**word**"], "undo the unbold");
    undo(&mut f);
    assert_eq!(f.texts(), ["word"], "undo the bold");
    undo(&mut f);
    assert_eq!(f.texts(), [""], "undo the typing");
    redo(&mut f);
    redo(&mut f);
    assert_eq!(f.texts(), ["**word**"]);
    redo(&mut f);
    assert_eq!(f.texts(), ["word"]);
}

#[test]
fn undo_splits_block_changes_pastes_and_images() {
    let mut f = Fixture::new();
    type_steadily(&mut f, "one");
    f.press(Key::Enter);
    type_steadily(&mut f, "two");
    assert_eq!(f.texts(), ["one", "two"]);
    undo(&mut f);
    assert_eq!(f.texts(), ["one", ""]);
    undo(&mut f);
    assert_eq!(f.texts(), ["one"], "the split is undone");

    // Turning a block into a heading.
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::ALT, Key::Num1);
    assert_eq!(f.kinds(), [BlockKind::Heading(1)]);
    undo(&mut f);
    assert_eq!(f.kinds(), [BlockKind::Paragraph]);

    // A pasted link, then a pasted picture.
    // A real ⌘V: the paste, then the key coming back up.
    f.harness
        .event(egui::Event::Paste(" https://docs.rs".into()));
    f.harness.event(egui::Event::Key {
        key: Key::V,
        physical_key: Some(Key::V),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    f.harness.run();
    assert_eq!(f.texts(), ["one[docs.rs](https://docs.rs)"]);
    paste_image(&mut f);
    assert_eq!(image_srcs(&f).len(), 1);
    undo(&mut f);
    assert!(image_srcs(&f).is_empty());
    undo(&mut f);
    assert_eq!(f.texts(), ["one"]);
}

fn indents(f: &Fixture) -> Vec<u8> {
    f.app().editor.doc.blocks.iter().map(|b| b.indent).collect()
}

#[test]
fn tab_indents_only_the_line_at_the_caret() {
    let mut f = Fixture::new();
    f.type_text("\u{201c}...that they must repent\u{201d}");
    f.shortcut_mods(egui::Modifiers::SHIFT, Key::Enter);
    f.press(Key::Tab);
    f.type_text("My note on it, long enough that it wraps onto another row so we can see the wrapped row hang at the indent too.");
    assert_eq!(
        f.texts(),
        ["\u{201c}...that they must repent\u{201d}\n\tMy note on it, long enough that it wraps onto another row so we can see the wrapped row hang at the indent too."]
    );
    // Shift+Enter again stays at the indent.
    f.shortcut_mods(egui::Modifiers::SHIFT, Key::Enter);
    f.type_text("More");
    assert!(f.texts()[0].ends_with("too.\n\tMore"));
    f.snapshot("indent");

    // Shift+Tab takes just this line back out.
    f.shortcut_mods(egui::Modifiers::SHIFT, Key::Tab);
    assert!(f.texts()[0].ends_with("too.\nMore"));
    f.press(Key::Tab);
    // Backspace where the line's text starts removes the indent.
    for _ in 0.."More".len() {
        f.press(Key::ArrowLeft);
    }
    f.press(Key::Backspace);
    assert!(f.texts()[0].ends_with("too.\nMore"));
    f.shortcut(Key::Z);
    assert!(f.texts()[0].ends_with("too.\n\tMore"));

    // It's saved as a tab on just that line.
    assert!(f.saved_markdown().contains("repent\u{201d}\n\tMy note"));
}

#[test]
fn tab_nests_list_items_and_indents_pictures() {
    let mut f = Fixture::new();
    f.type_text("- ");
    f.type_text("one");
    f.press(Key::Enter);
    f.press(Key::Tab);
    f.type_text("two");
    assert_eq!(f.kinds(), [BlockKind::Bullet, BlockKind::Bullet]);
    assert_eq!(indents(&f), [0, 1]);
    let md = f.saved_markdown();
    assert!(md.starts_with("- one\n\t- two"), "{md:?}");

    // A picture pasted on an indented line lines up with it.
    f.press(Key::Enter);
    f.press(Key::Enter); // An empty nested item moves out a level…
    assert_eq!(
        (f.kinds()[2].clone(), indents(&f)[2]),
        (BlockKind::Bullet, 0)
    );
    f.press(Key::Enter); // …then out of the list.
    assert_eq!(f.kinds()[2], BlockKind::Paragraph);
    f.press(Key::Tab);
    assert_eq!(f.texts()[2], "\t");
    paste_image(&mut f);
    assert!(
        matches!(f.kinds()[2], BlockKind::Image { .. }),
        "the indented empty line became the picture"
    );
    assert_eq!(indents(&f)[2], 1);
    assert_eq!(f.texts()[3], "\t", "typing continues at the indent below");
}

const PAGE: &str = r#"<html><head>
    <title>Grace | Gospel Library</title>
    <meta property="og:site_name" content="Gospel Library">
    <meta property="og:title" content="Grace">
    <meta name="author" content="David A. Bednar">
    <meta property="article:published_time" content="2023-04-02T10:00:00Z">
</head></html>"#;

fn fake_fetch(_url: &str) -> Result<String, String> {
    Ok(PAGE.to_string())
}

/// Types "/citation", picks it from the slash menu, and waits for the form.
fn open_citation_form(f: &mut Fixture) {
    f.harness.state_mut().editor.fetch_page = fake_fetch;
    f.type_text(" /citation");
    f.press(Key::Enter);
    assert!(
        f.harness.state_mut().editor.citation_form().is_some(),
        "form opened"
    );
}

/// Pastes a link into the form and waits for the page lookup to finish.
fn paste_link_and_wait(f: &mut Fixture, url: &str) {
    f.harness.event(egui::Event::Paste(url.into()));
    for _ in 0..100 {
        f.harness.run();
        let filled = f
            .harness
            .state_mut()
            .editor
            .citation_form()
            .is_some_and(|form| !form.source.title.is_empty());
        if filled {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the page lookup never filled the form");
}

#[test]
fn slash_citation_adds_a_numbered_citation_filled_from_a_link() {
    let mut f = Fixture::new();
    f.type_text("My grace is sufficient");
    open_citation_form(&mut f);
    paste_link_and_wait(
        &mut f,
        "https://www.churchofjesuschrist.org/study/grace?utm_source=x",
    );
    {
        let form = f.harness.state_mut().editor.citation_form().unwrap();
        assert_eq!(form.source.title, "Grace");
        assert_eq!(form.source.container, "Gospel Library");
        assert_eq!(form.source.authors, "David A. Bednar");
    }
    f.snapshot("citation_form");
    f.press(Key::Enter);

    assert!(f.harness.state_mut().editor.citation_form().is_none());
    assert_eq!(
        f.texts(),
        ["My grace is sufficient [^1]"],
        "the space before /citation stays"
    );
    let citations = &f.app().editor.doc.citations;
    assert_eq!(citations.len(), 1);
    assert!(
        citations[0]
            .text
            .starts_with(
                "Bednar, David A. \u{201c}Grace.\u{201d} *Gospel Library*, 2 Apr. 2023, \
                 [www.churchofjesuschrist.org/study/grace](https://www.churchofjesuschrist.org/study/grace)."
            ),
        "{}",
        citations[0].text
    );
    // Typing carries on right after the number.
    f.type_text(".");
    assert_eq!(f.texts(), ["My grace is sufficient [^1]."]);
    f.snapshot("citation");

    let md = f.saved_markdown();
    assert!(
        md.starts_with("My grace is sufficient [^1].\n\n[^1]: Bednar"),
        "{md}"
    );
}

#[test]
fn slash_scripture_cites_the_reference_instead_of_a_number() {
    let mut f = Fixture::new();
    f.type_text("The brother of Jared");
    f.type_text(" /scripture");
    f.press(Key::Enter);
    f.snapshot("scripture_picker");
    f.type_text("Ether 2:1-4");
    f.wait_for_scripture_index();
    f.wait_for_scripture_result("Ether 2:1-4");
    f.snapshot("scripture_picker_results");
    f.harness.ctx.set_theme(egui::Theme::Light);
    f.harness.run();
    f.snapshot("scripture_picker_results_light");
    f.harness.ctx.set_theme(egui::Theme::Dark);
    f.harness.run();
    f.press(Key::Enter);

    assert_eq!(f.texts(), ["The brother of Jared [^Ether 2:1-4]"]);
    let citations = &f.app().editor.doc.citations;
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].id, "Ether 2:1-4");
    assert!(citations[0].text.contains("Jared"), "{}", citations[0].text);
    assert!(
        citations[0].text.contains("Nimrod"),
        "{}",
        citations[0].text
    );
    f.type_text(".");
    assert_eq!(f.texts(), ["The brother of Jared [^Ether 2:1-4]."]);

    // A single verse, and a numbered citation still counts on its own.
    f.press(Key::Enter);
    f.type_text("He read");
    f.type_text(" /scripture");
    f.press(Key::Enter);
    f.type_text("1 Nephi 1:11");
    f.wait_for_scripture_result("1 Nephi 1:11");
    f.press(Key::Enter);
    assert_eq!(
        f.texts(),
        [
            "The brother of Jared [^Ether 2:1-4].",
            "He read [^1 Nephi 1:11]"
        ]
    );
    assert_eq!(f.app().editor.doc.citations[1].id, "1 Nephi 1:11");
    assert!(
        f.app().editor.doc.citations[1]
            .text
            .contains("gave unto him a book"),
        "{}",
        f.app().editor.doc.citations[1].text
    );
}

#[test]
fn slash_scripture_filters_each_keystroke_without_collapsing_the_popup() {
    let mut f = Fixture::new();
    f.wait_for_scripture_index();
    f.type_text("Read /scripture");
    f.press(Key::Enter);
    f.type_text("1 N");
    assert!(f.harness.query_by_label("1 Nephi 1:1").is_some());
    f.snapshot("scripture_live_book");
    let row = f.harness.get_by_label("1 Nephi 1:1").rect();
    let mut slowest_frame = Duration::ZERO;
    for letter in "ephi 1:1".chars() {
        let start = std::time::Instant::now();
        f.type_text(&letter.to_string());
        slowest_frame = slowest_frame.max(start.elapsed());
        // Results exist in the same input frame; no worker wait is needed.
        let first = f.harness.get_by_label("1 Nephi 1:1").rect();
        assert!((first.top() - row.top()).abs() < 1.0);
    }
    f.type_text("1");
    assert!(f.harness.query_by_label("1 Nephi 1:11").is_some());
    assert!((f.harness.get_by_label("1 Nephi 1:11").rect().top() - row.top()).abs() < 1.0);
    f.snapshot("scripture_live_verse");
    eprintln!("Rapid reference typing: slowest harness update {slowest_frame:?}");
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["Read [^1 Nephi 1:11]"]);
}

#[test]
fn slash_scripture_word_search_replaces_results_and_inserts_the_latest_match() {
    let mut f = Fixture::new();
    f.wait_for_scripture_index();
    f.type_text("Read /scripture");
    f.press(Key::Enter);
    f.type_text("in the beginning god created");
    f.wait_for_scripture_result("Genesis 1:1");
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["Read [^Genesis 1:1]"]);
}

#[test]
fn slash_file_link_inserts_a_title_that_opens_the_note() {
    let mut f = Fixture::with_notes(&[
        ("My Notes/grace", "# Grace\n\nA talk about grace.\n"),
        ("journal", "# Journal\n\n"),
    ]);
    assert_eq!(f.app().current, "journal");
    f.press(Key::Enter);
    f.type_text("See /file-link");
    f.press(Key::Enter);
    assert!(
        f.harness.get_all_by_label("Grace").count() >= 2,
        "the document list opened beside the sidebar"
    );
    f.snapshot("file_link_picker");
    f.press(Key::Escape);
    assert_eq!(f.texts(), ["Journal", "See "], "cancel removes the command");

    f.type_text("/file-link");
    f.press(Key::Enter);
    f.type_text("Grace");
    f.snapshot("file_link_picker_results");
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["Journal", "See [Grace](note:My%20Notes/grace)"]);

    let text = "See [Grace](note:My%20Notes/grace)";
    let pos = block_rect(&f, text).left_center() + egui::vec2(50.0, 0.0);
    f.harness.hover_at(pos);
    f.harness.run();
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.step();
    }
    f.harness.run();
    assert_eq!(f.app().current, "My Notes/grace");
    assert_eq!(f.texts(), ["Grace", "A talk about grace."]);
    let journal = fs::read_to_string(f.dir.path().join("journal.md")).unwrap();
    assert!(
        journal.contains("[Grace](note:My%20Notes/grace)"),
        "{journal}"
    );
}

#[test]
fn a_citation_added_before_others_renumbers_them() {
    let mut f = Fixture::with_note(
        "First thought\n\nSecond thought[^1]\n\n[^1]: Smith, Jane. *Later Book*. 2001.\n",
    );
    let end_of_first = "First thought".len();
    f.harness.state_mut().editor.place_caret(0, end_of_first);
    f.harness.run();
    open_citation_form(&mut f);
    {
        let form = f.harness.state_mut().editor.citation_form().unwrap();
        form.source.title = "Earlier Page".into();
        form.source.accessed = String::new();
    }
    f.harness.run();
    f.press(Key::Enter);

    assert_eq!(f.texts(), ["First thought [^1]", "Second thought[^2]"]);
    let list: Vec<(String, String)> = f
        .app()
        .editor
        .doc
        .citations
        .iter()
        .map(|c| (c.id.clone(), c.text.clone()))
        .collect();
    assert_eq!(
        list,
        [
            ("1".to_string(), "\u{201c}Earlier Page.\u{201d}".to_string()),
            (
                "2".to_string(),
                "Smith, Jane. *Later Book*. 2001.".to_string()
            ),
        ]
    );
}

#[test]
fn copying_a_superscript_inside_a_block_brings_its_citation() {
    let mut f = Fixture::with_note("City[^1] was besieged.\n\n[^1]: Tablets.\n");
    f.harness
        .state_mut()
        .editor
        .place_caret(0, "City[^1]".len());
    f.harness.run();
    // The hidden `[^` `]` have no width, so one arrow selects the superscript.
    f.harness
        .key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowLeft);
    f.harness.run();
    let copied = clipboard_after(&mut f, egui::Event::Copy).unwrap();
    assert_eq!(copied, "[^1]\n\n[^1]: Tablets.");

    f.harness
        .state_mut()
        .editor
        .place_caret(0, "City[^1] was besieged.".len());
    f.harness.run();
    f.harness.event(egui::Event::Paste(copied));
    f.harness.run();
    assert_eq!(f.texts(), ["City[^1] was besieged.[^1]"]);
    assert_eq!(f.app().editor.doc.citations.len(), 1);
    assert_eq!(f.app().editor.doc.citations[0].text, "Tablets.");
}

#[test]
fn pasting_a_copied_list_into_another_note_brings_its_citations() {
    let mut f = Fixture::with_notes(&[
        ("page", ""),
        (
            "sources",
            "1. The city was besieged[^1]\n2. Lehi saw it destroyed[^1 Nephi 1:4]\n\n[^1]: Tablets.\n[^1 Nephi 1:4]: Jerusalem.\n",
        ),
    ]);
    f.shortcut(Key::A);
    let copied = clipboard_after(&mut f, egui::Event::Copy).unwrap();
    assert!(copied.contains("[^1]: Tablets."), "{copied}");
    assert!(copied.contains("[^1 Nephi 1:4]: Jerusalem."), "{copied}");

    f.click("Untitled");
    assert_eq!(f.app().current, "page");
    f.harness.event(egui::Event::Paste(copied));
    f.harness.run();
    assert_eq!(
        f.texts(),
        [
            "The city was besieged[^1]",
            "Lehi saw it destroyed[^1 Nephi 1:4]"
        ]
    );
    let list: Vec<_> = f
        .app()
        .editor
        .doc
        .citations
        .iter()
        .map(|c| (c.id.as_str(), c.text.as_str()))
        .collect();
    assert_eq!(list, [("1", "Tablets."), ("1 Nephi 1:4", "Jerusalem.")]);
}

#[test]
fn clicking_a_citation_number_highlights_it_in_the_list() {
    let mut f = Fixture::with_note(
        "Faith precedes the miracle[^1] and hope[^2].\n\n[^1]: \u{201c}Faith.\u{201d} *Site*.\n[^2]: \u{201c}Hope.\u{201d} *Site*.\n",
    );
    f.harness.run();
    assert!(
        f.harness.query_by_label("Citation 2").is_none(),
        "the list starts collapsed"
    );
    assert!(f.harness.query_by_label("Citations").is_some());
    let number = f
        .app()
        .editor
        .citation_number_rect("2")
        .expect("number drawn");
    let pos = number.center();
    f.harness.hover_at(pos);
    f.harness.run();
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.run();
    }
    assert_eq!(f.app().editor.highlighted_citation(), Some("2"));
    assert!(f.harness.query_by_label("Citation 2").is_some());
    f.snapshot("citation_highlight");

    // Backspace right after a number removes the whole citation.
    f.harness
        .state_mut()
        .editor
        .place_caret(0, "Faith precedes the miracle[^1]".len());
    f.harness.run();
    f.press(Key::Backspace);
    assert_eq!(f.texts(), ["Faith precedes the miracle and hope[^1]."]);
    assert_eq!(f.app().editor.doc.citations.len(), 1);
    assert_eq!(
        f.app().editor.doc.citations[0].text,
        "\u{201c}Hope.\u{201d} *Site*."
    );
    // …and undo brings it back.
    f.shortcut(Key::Z);
    assert_eq!(f.app().editor.doc.citations.len(), 2);
}

#[test]
fn clicking_the_citations_header_opens_and_closes_the_list() {
    let mut f = Fixture::with_note(
        "Faith precedes the miracle[^1].\n\n[^1]: \u{201c}Faith.\u{201d} *Site*.\n",
    );
    f.harness.run();
    assert!(f.harness.query_by_label("Citation 1").is_none());
    f.click("Citations");
    assert!(f.harness.query_by_label("Citation 1").is_some());
    f.click("Citations");
    assert!(
        f.harness.query_by_label("Citation 1").is_none(),
        "clicking the header again closes the list"
    );
}

#[test]
fn citation_values_can_be_selected_and_copied() {
    let mut f = Fixture::with_note(
        "Faith precedes the miracle[^1].\n\n[^1]: Tablets are preserved in the archive.\n",
    );
    f.harness.run();
    f.click("Citations");

    let row = f.harness.get_by_label("Citation 1").rect();
    let from = row.left_center() + egui::vec2(34.0, 0.0);
    let to = from + egui::vec2(70.0, 0.0);
    f.harness.hover_at(from);
    f.harness.run();
    f.harness.drag_at(from);
    f.harness.run();
    f.harness.hover_at(to);
    f.harness.run();
    f.harness.drop_at(to);
    f.harness.run();

    let copied = clipboard_after(&mut f, egui::Event::Copy).expect("citation selection");
    assert!(copied.starts_with("Tablets ar"), "{copied:?}");
}

fn click_with(f: &mut Fixture, pos: egui::Pos2, button: egui::PointerButton) {
    f.harness.hover_at(pos);
    f.harness.run();
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.run();
    }
}

const LINKED: &str = "Read [the talk](https://example.org/talk?utm_source=x) today";

/// Opens the link menu by right-clicking "the talk".
fn right_click_link(f: &mut Fixture) {
    let pos = block_rect(f, LINKED).left_center() + egui::vec2(60.0, 0.0);
    click_with(f, pos, egui::PointerButton::Secondary);
}

#[test]
fn right_clicking_a_link_offers_its_menu() {
    let mut f = Fixture::with_note(&format!("{LINKED}\n"));
    right_click_link(&mut f);
    for item in [
        "Copy link address",
        "Edit link",
        "Convert to citation",
        "Remove link",
    ] {
        assert!(f.harness.query_by_label(item).is_some(), "{item} missing");
    }
    f.snapshot("link_menu");

    // Copy puts the address on the clipboard.
    f.harness.get_by_label("Copy link address").click();
    f.harness.step();
    let copied = f.harness.output().platform_output.commands.iter().any(|c| {
        matches!(c, egui::OutputCommand::CopyText(t) if t == "https://example.org/talk?utm_source=x")
    });
    assert!(copied);
    f.harness.run();
    assert!(
        f.harness.query_by_label("Edit link").is_none(),
        "menu closed"
    );

    // Remove keeps the words.
    right_click_link(&mut f);
    f.click("Remove link");
    assert_eq!(f.texts(), ["Read the talk today"]);
}

#[test]
fn edit_link_changes_its_text_and_address() {
    let mut f = Fixture::with_note(&format!("{LINKED}\n"));
    right_click_link(&mut f);
    f.click("Edit link");
    // The text field is focused with the link's words; replace them.
    f.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    f.type_text("Elder Bednar's talk");
    f.snapshot("link_form");
    f.press(Key::Tab);
    f.harness
        .key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    f.type_text("https://example.org/other");
    f.press(Key::Enter);
    assert_eq!(
        f.texts(),
        ["Read [Elder Bednar's talk](https://example.org/other) today"]
    );
}

#[test]
fn convert_a_link_to_a_citation() {
    let mut f = Fixture::with_note(&format!("{LINKED}\n"));
    f.harness.state_mut().editor.fetch_page = fake_fetch;
    right_click_link(&mut f);
    f.click("Convert to citation");
    {
        let form = f
            .harness
            .state_mut()
            .editor
            .citation_form()
            .expect("form opened");
        assert_eq!(form.source.url, "https://example.org/talk?utm_source=x");
        assert_eq!(form.source.title, "the talk");
    }
    // The lookup fills in what the link didn't say.
    for _ in 0..100 {
        f.harness.run();
        let filled = f
            .harness
            .state_mut()
            .editor
            .citation_form()
            .is_some_and(|form| !form.source.authors.is_empty());
        if filled {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["Read the talk[^1] today"]);
    let citation = &f.app().editor.doc.citations[0].text;
    assert!(
        citation.starts_with("Bednar, David A. \u{201c}the talk.\u{201d} *Gospel Library*"),
        "{citation}"
    );
    assert!(
        citation.contains("(https://example.org/talk)"),
        "cleaned link: {citation}"
    );
}

#[test]
fn a_bare_address_converts_to_just_a_number() {
    let text = "See https://example.org/page for more";
    let mut f = Fixture::with_note(&format!("{text}\n"));
    f.harness.state_mut().editor.fetch_page = fake_fetch;
    let pos = block_rect(&f, text).left_center() + egui::vec2(80.0, 0.0);
    click_with(&mut f, pos, egui::PointerButton::Secondary);
    // A typed-out address is always a link, so Remove does nothing.
    f.click("Remove link");
    assert_eq!(f.texts(), [text]);
    assert!(
        f.harness.query_by_label("Convert to citation").is_some(),
        "menu still open"
    );
    f.click("Convert to citation");
    for _ in 0..100 {
        f.harness.run();
        let filled = f
            .harness
            .state_mut()
            .editor
            .citation_form()
            .is_some_and(|form| !form.source.title.is_empty());
        if filled {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f.press(Key::Enter);
    assert_eq!(f.texts(), ["See[^1] for more"]);
}

#[test]
fn scriptures_page_opens_a_book_then_its_chapter() {
    let mut f = Fixture::with_note("# Note\n\nKeep me.\n");
    f.click("Scriptures");
    assert!(f.app().scriptures.is_open());
    // The book icon takes over the sidebar. Notes is no longer the selected rail item.
    assert!(f.harness.query_by_label("Hide notes").is_none());
    assert!(f.harness.query_by_label("Show notes").is_some());
    assert!(f.harness.query_by_label("Genesis").is_some());
    f.snapshot("scriptures");
    assert!(f.harness.query_by_label("1 Nephi").is_some());
    assert!(f.harness.query_by_label("Moses").is_some());

    f.click("Genesis");
    assert!(f.harness.query_by_label("Chapter 1").is_some());
    assert!(f.harness.query_by_label("Chapter 50").is_some());
    f.snapshot("scriptures_chapters");

    f.click("Chapter 1");
    let verse = "In the beginning God created the heaven and the earth.";
    let last = "And God saw every thing that he had made, and, behold, it was very good. And the evening and the morning were the sixth day.";
    assert!(f.harness.query_by_label(verse).is_some());
    assert!(
        f.harness.query_by_label(last).is_some(),
        "the whole chapter is on the page"
    );
    assert!(f.harness.query_by_label("Chapter 2").is_none());
    f.snapshot("scriptures_chapter");

    // Typing stays out of the note underneath.
    f.type_text("hello");
    assert_eq!(f.texts(), ["Note", "Keep me."]);

    f.click("Next chapter");
    assert!(f
        .harness
        .query_by_label("Thus the heavens and the earth were finished, and all the host of them.")
        .is_some());
    f.click("Previous chapter");
    assert!(f.harness.query_by_label(verse).is_some());

    f.click("Back");
    assert!(f.harness.query_by_label("Chapter 2").is_some());
    f.press(Key::Escape);
    assert!(f.harness.query_by_label("Exodus").is_some());
    f.press(Key::Escape);
    assert!(!f.app().scriptures.is_open());
    assert_eq!(f.texts(), ["Note", "Keep me."]);

    // The book icon returns to the same place, and a note leaves it.
    f.click("Scriptures");
    assert!(f.harness.query_by_label("Exodus").is_some());
    f.harness
        .get_by_label("Doctrine and Covenants")
        .scroll_to_me();
    f.harness.run();
    f.click("Doctrine and Covenants");
    assert!(f.harness.query_by_label("Section 4").is_some());
    assert!(f.harness.query_by_label("Section 138").is_some());
    f.click("Section 4");
    assert!(f
        .harness
        .query_by_label(
            "Now behold, a marvelous work is about to come forth among the children of men."
        )
        .is_some());
    f.click("Show notes");
    assert!(!f.app().scriptures.is_open());
    assert!(f.harness.query_by_label("Hide notes").is_some());
    assert!(f.harness.query_by_label("Genesis").is_none());
    assert_eq!(f.texts(), ["Note", "Keep me."]);
}

#[test]
fn right_clicking_a_verse_offers_its_reference() {
    let mut f = Fixture::with_note("# Note\n");
    f.click("Scriptures");
    f.click("Genesis");
    f.click("Chapter 1");

    let verse = f
        .harness
        .get_by_label("In the beginning God created the heaven and the earth.")
        .rect()
        .center();
    for pressed in [true, false] {
        f.harness.event(egui::Event::PointerButton {
            pos: verse,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        f.harness.run();
    }
    assert!(f.harness.query_by_label("Copy reference").is_some());

    f.harness.get_by_label("Copy reference").click();
    f.harness.step();
    let copied =
        f.harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            });
    assert_eq!(copied, Some("Genesis 1:1"));
    f.harness.run();
    assert!(f.harness.query_by_label("Copy reference").is_none());
}

#[test]
fn right_clicking_a_scripture_citation_opens_and_highlights_it() {
    let mut f = Fixture::with_note(
        "Read this[^Ether 2:1-4].\n\n[^Ether 2:1-4]: And it came to pass that the Lord spoke unto Jared.\n",
    );
    let citation = f
        .app()
        .editor
        .citation_number_rect("Ether 2:1-4")
        .expect("scripture citation drawn")
        .center();
    click_with(&mut f, citation, egui::PointerButton::Secondary);
    assert!(f.harness.query_by_label("Go to Scripture").is_some());

    f.click("Go to Scripture");

    assert!(f.app().scriptures.is_open());
    assert_eq!(f.app().scriptures.title().as_deref(), Some("Ether 2"));
    assert_eq!(f.app().scriptures.highlighted_verse_range(), Some((1, 4)));
}

#[test]
fn right_clicking_a_scripture_citation_in_the_list_offers_navigation() {
    let mut f = Fixture::with_note(
        "Read this[^Ether 2:1-4].\n\n[^Ether 2:1-4]: And it came to pass that the Lord spoke unto Jared.\n",
    );
    f.click("Citations");
    let citation = f
        .harness
        .get_by_label("Citation Ether 2:1-4")
        .rect()
        .center();
    click_with(&mut f, citation, egui::PointerButton::Secondary);

    assert!(f.harness.query_by_label("Go to Scripture").is_some());
}

#[test]
fn editing_a_scripture_citation_replaces_its_reference_and_text() {
    let mut f = Fixture::with_note(
        "Read this[^Ether 2:1-4] twice[^Ether 2:1-4].\n\n[^Ether 2:1-4]: And it came to pass that the Lord spoke unto Jared.\n",
    );
    f.click("Citations");
    let citation = f
        .harness
        .get_by_label("Citation Ether 2:1-4")
        .rect()
        .center();
    click_with(&mut f, citation, egui::PointerButton::Secondary);
    f.click("Edit");

    f.shortcut(Key::A);
    f.type_text("1 Nephi 1:1");
    f.wait_for_scripture_index();
    f.press(Key::Enter);

    assert_eq!(f.texts(), ["Read this[^1 Nephi 1:1] twice[^1 Nephi 1:1]."]);
    assert_eq!(f.app().editor.doc.citations[0].id, "1 Nephi 1:1");
    assert!(f.app().editor.doc.citations[0].text.contains("Nephi"));
}

#[test]
fn cmd_f_finds_in_the_open_chapter() {
    let mut f = Fixture::with_note("# Note\n\nKeep me.\n");
    f.click("Scriptures");
    f.click("Genesis");
    f.click("Chapter 1");
    let verse = "In the beginning God created the heaven and the earth.";

    f.shortcut(Key::F);
    assert!(f.app().scriptures.chapter_find.open);
    assert!(!f.app().editor.find.open, "Cmd+F stays on the chapter");
    assert!(f
        .harness
        .query_by(|node| node.placeholder() == Some("Find in chapter"))
        .is_some());

    f.type_text("firmament");
    assert_eq!(f.app().scriptures.chapter_find.matches.len(), 9);
    assert!(shows_count(&f, "1 of 9"), "expected 1 of 9");
    f.snapshot("scriptures_find");

    f.press(Key::Enter);
    assert!(shows_count(&f, "2 of 9"), "expected 2 of 9");
    f.click("Previous match");
    assert_eq!(f.app().scriptures.chapter_find.current, 0);

    // A typo matches only with fuzzy on, and the field keeps focus.
    f.shortcut(Key::F);
    f.type_text("firmamant");
    assert_eq!(f.app().scriptures.chapter_find.query, "firmamant");
    assert!(shows_count(&f, "No results"), "expected No results");
    f.click("Fuzzy");
    assert!(f.app().scriptures.chapter_find.fuzzy);
    assert!(
        !f.app().scriptures.chapter_find.matches.is_empty(),
        "fuzzy finds firmament"
    );
    f.snapshot("scriptures_find_fuzzy");

    // Escape closes the bar and stays on the chapter.
    f.press(Key::Escape);
    assert!(!f.app().scriptures.chapter_find.open);
    assert!(f.harness.query_by_label(verse).is_some());
    assert_eq!(f.texts(), ["Note", "Keep me."]);

    // The next chapter is a new search.
    f.shortcut(Key::F);
    f.type_text("firmament");
    assert!(shows_count(&f, "1 of 9"), "expected 1 of 9");
    f.click("Next chapter");
    assert!(shows_count(&f, "No results"), "Genesis 2 has no firmament");
}

#[test]
fn scriptures_filter_and_search() {
    let mut f = Fixture::with_note("# Note\n\nKeep me.\n");
    f.click("Scriptures");

    f.harness
        .get_by(|node| node.placeholder() == Some("Filter books"))
        .click();
    f.harness.run();
    f.type_text("gene");
    assert!(f.harness.query_by_label("Genesis").is_some());
    assert!(
        f.harness.query_by_label("Exodus").is_none(),
        "the book list narrows as you type"
    );
    f.press(Key::Escape);
    assert!(
        f.harness.query_by_label("Exodus").is_some(),
        "escape clears the filter"
    );

    f.harness
        .get_by(|node| node.placeholder() == Some("Search scriptures"))
        .click();
    f.harness.run();
    f.type_text("in the beginning god created");
    assert!(f.harness.query_by_label("Genesis 1:1").is_some());
    f.click("Genesis 1:1");
    assert!(f
        .harness
        .query_by_label("In the beginning God created the heaven and the earth.")
        .is_some());
    assert_eq!(f.texts(), ["Note", "Keep me."]);
}

#[test]
fn scripture_search_selects_and_reveals_a_verse_range() {
    let mut f = Fixture::with_note("# Note\n\nKeep me.\n");
    f.click("Scriptures");
    f.harness
        .get_by(|node| node.placeholder() == Some("Search scriptures"))
        .click();
    f.harness.run();
    f.type_text("Ether 2:1-4");
    f.click("Ether 2:1-4");

    assert_eq!(f.app().scriptures.highlighted_verse_range(), Some((1, 4)));
    assert_eq!(f.app().scriptures.title().as_deref(), Some("Ether 2"));
}

#[test]
fn scripture_search_keeps_more_than_ten_results_and_mounts_a_page() {
    let mut f = Fixture::with_note("# Note\n\nKeep me.\n");
    f.click("Scriptures");
    f.harness
        .get_by(|node| node.placeholder() == Some("Search scriptures"))
        .click();
    f.harness.run();
    f.type_text("and the");

    let results = scriptures::find_verses("and the");
    assert!(results.len() > 10);
    assert!(f
        .harness
        .query_by_label(&format!("Results ({})", results.len()))
        .is_some());
    let eleventh = results[10].label.clone();
    assert!(f.harness.query_by_label(&eleventh).is_some());
}

#[test]
fn right_clicking_a_misspelling_offers_a_fix_ignore_all_and_the_dictionary() {
    let mut f = Fixture::with_note("helo there\n\nhelo again\n");
    let path = f.dir.path().join("dictionary");
    f.harness.state_mut().spelling = crate::spell::Spelling::for_test(
        Some(path.clone()),
        &["there", "again"],
        &[("helo", &["hello"])],
    );
    f.harness.run();

    let pos = block_rect(&f, "helo there").left_center() + egui::vec2(12.0, 0.0);
    click_with(&mut f, pos, egui::PointerButton::Secondary);
    for item in ["hello", "Ignore All", "Add to Dictionary"] {
        assert!(f.harness.query_by_label(item).is_some(), "{item} missing");
    }

    f.click_menu_item("hello");
    assert_eq!(f.texts(), ["hello there", "helo again"]);
    assert!(
        f.harness.query_by_label("Ignore All").is_none(),
        "the menu closed"
    );

    let pos = block_rect(&f, "helo again").left_center() + egui::vec2(12.0, 0.0);
    click_with(&mut f, pos, egui::PointerButton::Secondary);
    f.click_menu_item("Add to Dictionary");
    assert!(
        f.harness
            .state_mut()
            .spelling
            .misspellings("helo again")
            .is_empty(),
        "the added word is no longer misspelled"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "helo\n");

    // Ignore All covers every later copy, not just the one that was clicked.
    f.harness.state_mut().spelling =
        crate::spell::Spelling::for_test(None, &["there", "again"], &[("helo", &["hello"])]);
    f.harness.run();
    click_with(&mut f, pos, egui::PointerButton::Secondary);
    f.click_menu_item("Ignore All");
    let spelling = &mut f.harness.state_mut().spelling;
    assert!(spelling.misspellings("helo again").is_empty());
    assert!(spelling.misspellings("helo there").is_empty());
}

#[test]
fn highlight_and_underline_save_and_come_back() {
    let mut f = Fixture::new();
    type_steadily(&mut f, "hope");
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::H);
    assert_eq!(f.texts(), ["<mark #FFE08A>hope</mark>"]);
    f.shortcut_mods(egui::Modifiers::COMMAND, Key::U);
    assert_eq!(f.texts(), ["<u #9EC7F5><mark #FFE08A>hope</mark></u>"]);
    // The same highlight shortcut removes only the highlight.
    f.shortcut_mods(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::H);
    assert_eq!(f.texts(), ["<u #9EC7F5>hope</u>"]);
    let saved = f.saved_markdown();
    assert!(saved.contains("<u #9EC7F5>hope</u>"), "{saved}");

    let reopened = Fixture::with_note(&saved);
    assert_eq!(reopened.texts(), ["<u #9EC7F5>hope</u>"]);
}

#[test]
fn opening_a_folder_loads_annotations() {
    let markdown = "A <mark #F5B3CE>hope</mark> and <u #9EC7F5>faith</u>.\n";
    let f = Fixture::with_notes(&[("Notes/study", markdown)]);
    assert_eq!(
        f.texts(),
        ["A <mark #F5B3CE>hope</mark> and <u #9EC7F5>faith</u>."]
    );
}

#[test]
fn selecting_text_shows_the_highlight_bar() {
    let mut f = Fixture::with_note("hope\n");
    for _ in 0.."hope".len() {
        f.shortcut_mods(egui::Modifiers::SHIFT, Key::ArrowLeft);
    }
    assert!(f.harness.query_by_label("Highlight").is_some());
    assert!(f.harness.query_by_label("Underline").is_some());
    assert!(f.harness.query_by_label("Clear").is_some());
    assert!(
        f.harness.query_by_label("Color Pink").is_some(),
        "the color strip is part of the bar"
    );

    f.click_menu_item("Color Pink");
    assert_eq!(f.texts(), ["<mark #F5B3CE>hope</mark>"]);
    assert!(
        f.harness.query_by_label("Underline").is_some(),
        "the selection stays, so the bar stays"
    );

    f.click_menu_item("Underline");
    assert!(f.texts()[0].contains("<u #F5B3CE>"), "{:?}", f.texts());

    f.click_menu_item("Clear");
    assert_eq!(f.texts(), ["hope"]);
}

#[test]
fn moving_selected_text_cuts_it_from_source_and_inserts_it_into_destination() {
    let mut f = Fixture::with_notes(&[
        ("destination", "# Destination\n\nExisting text.\n"),
        ("source", "# Source\n\nMove this text.\n"),
    ]);

    // The caret starts at the end of the source note. Select "Move this text.".
    for _ in 0..15 {
        f.shortcut_mods(egui::Modifiers::SHIFT, Key::ArrowLeft);
    }
    f.click("Move To");
    assert!(f.harness.get_all_by_label("Destination").count() >= 2);
    assert!(selection(&f).is_some());
    f.press(Key::Enter);
    assert!(selection(&f).is_none());
    assert!(f.app().error.is_none(), "{:?}", f.app().error);

    assert_eq!(f.app().current, "destination");
    assert_eq!(
        f.texts(),
        ["Destination", "Existing text.", "Move this text."]
    );

    let source = fs::read_to_string(f.dir.path().join("source.md")).unwrap();
    let destination = fs::read_to_string(f.dir.path().join("destination.md")).unwrap();
    assert!(!source.contains("Move this text."), "{source}");
    assert!(
        destination.contains("Existing text.\n\nMove this text."),
        "{destination}"
    );
}
