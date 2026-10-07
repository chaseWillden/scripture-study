//! The block editor: one borderless text field per block, with the slash menu.
//!
//! Editing semantics live in `scripture_study_core::editor`; this module maps keys and
//! clicks onto those operations and draws the result.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self,
    text::{CCursor, CCursorRange},
    vec2, Color32, CornerRadius, FontId, Frame, Galley, Id, Key, KeyboardShortcut, Margin,
    Modifiers, Order, Pos2, Rect, RichText, Sense, Shadow, Shape, Stroke, TextBuffer, TextEdit, Ui,
};
use scripture_study_core::{
    citations,
    commands::{self, Action, Command},
    document::outline_key,
    editor::{self as ops, Caret, SlashQuery},
    history::History,
    inline::{self, MarkKind, DEFAULT_HIGHLIGHT, DEFAULT_UNDERLINE},
    links, marks, scriptures,
    selection::{self, Selection},
    settings::CollapsedOutline,
    Block, BlockKind, Document, NoteMeta,
};

use crate::citation_form::{self, CitationForm, Outcome};
use crate::file_menu::{FilePicker, MovePicker};
use crate::find_bar::{self, FindBar};
use crate::link_menu::{self, LinkForm, LinkMenu, LinkTarget};
use crate::marks::{BarAction, MarkMenu};
use crate::meta::{self, MetaEditor};
use crate::scripture_menu::ScripturePicker;
use crate::spell::{self, SpellChoice, SpellMenu, Spelling};
use crate::theme::{self, Palette};

const MENU_WIDTH: f32 = 280.0;
const MENU_ROW_HEIGHT: f32 = 32.0;
const QUOTE_INDENT: f32 = 18.0;
/// How far each indent level (Tab) moves a block right.
pub const INDENT_WIDTH: f32 = theme::INDENT_WIDTH;
/// Space between a paragraph's number in the margin and its text.
const PARAGRAPH_NUMBER_GAP: f32 = 14.0;
/// Tall images are scaled down to fit this height.
const IMAGE_MAX_HEIGHT: f32 = 560.0;
pub const EMPTY_NOTE_HINT: &str = "Start writing, or type '/' for commands";

/// Something the app needs to react to.
pub enum Event {
    Changed,
    /// A slash command that acts on notes rather than on blocks.
    Run(Action),
    /// Open the note with this id.
    OpenNote(String),
    /// Open a scripture citation in the reader.
    OpenScripture(String),
    /// The current selection was moved into another page.
    MoveSelection {
        destination: String,
        markdown: String,
    },
}

enum Op {
    Split {
        block: usize,
        start: usize,
        end: usize,
    },
    Backspace(usize),
    DeleteForward(usize),
    Focus(Caret),
    Run {
        block: usize,
        query: SlashQuery,
        action: Action,
    },
    ToggleTodo(usize),
    ExitCode(usize),
    FocusEnd,
    Style {
        block: usize,
        start: usize,
        end: usize,
        marker: &'static str,
    },
    /// Highlight or underline. `selection` uses the document selection;
    /// otherwise `start..end` in `block`.
    Mark {
        selection: bool,
        block: usize,
        start: usize,
        end: usize,
        kind: MarkKind,
        color: Option<u32>,
        toggle: bool,
    },
    /// Take both highlight and underline off a selection.
    ClearMarks {
        selection: bool,
        block: usize,
        start: usize,
        end: usize,
    },
    Kind {
        block: usize,
        kind: BlockKind,
        start: usize,
        end: usize,
    },
    /// Start or change a selection that spans blocks.
    Select(Selection),
    DeleteSelection,
    ReplaceSelection(String),
    /// Insert `text` in place of characters `start..end` of one block. A
    /// multi-line paste comes through here so a list stays a list.
    InsertAt {
        block: usize,
        start: usize,
        end: usize,
        text: String,
    },
    SplitSelection,
    /// Turn a pasted URL into a link, around the selected text if any.
    InsertLink {
        block: usize,
        start: usize,
        end: usize,
        url: String,
    },
    SelectImage(usize),
    RemoveBlock(usize),
    /// Put the caret in the nearest text block after (or before) `block`,
    /// adding one at the end if needed.
    FocusBeside {
        block: usize,
        forward: bool,
    },
    /// Tab / Shift+Tab over blocks `first..=last`, keeping the caret (or
    /// selection) `start..end` in `first` where it was.
    Indent {
        first: usize,
        last: usize,
        deeper: bool,
        start: usize,
        end: usize,
    },
    /// Turn a link into plain text followed by a new citation.
    ConvertLinkToCitation {
        target: LinkTarget,
        text: String,
    },
    /// Add a citation (formatted text) at a place in a block.
    InsertCitation {
        block: usize,
        char: usize,
        text: String,
    },
    /// Cite a scripture. `label` is the superscript (`Ether 2:1-4`).
    InsertScripture {
        block: usize,
        at: usize,
        label: String,
        text: String,
    },
    /// Replace a scripture citation everywhere it appears in the note.
    EditScripture {
        old_label: String,
        new_label: String,
        text: String,
    },
    /// Insert a link to another note. `title` is the visible text.
    InsertNoteLink {
        block: usize,
        at: usize,
        id: String,
        title: String,
    },
    MoveSelection {
        destination: String,
    },
    /// Replace a block's text and select `char..end` (a caret if equal).
    SetText {
        block: usize,
        text: String,
        char: usize,
        end: usize,
    },
}

struct OpenMenu<'a> {
    block: usize,
    query: SlashQuery,
    matches: Vec<&'a Command>,
    anchor: Pos2,
}

/// What a click in the text can lead to.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Url(String),
    /// Another note in the library.
    Note(String),
    /// A citation's number: jump to it in the list below.
    Citation(String),
}

/// A press on a link, which opens it if it turns into a click.
struct LinkPress {
    block: usize,
    target: Target,
    /// Whether the block was being edited when pressed.
    editing: bool,
}

struct ScriptureCitationMenu {
    reference: String,
    pos: Pos2,
    just_opened: bool,
}

impl ScriptureCitationMenu {
    fn new(reference: String, pos: Pos2) -> Self {
        Self {
            reference,
            pos,
            just_opened: true,
        }
    }

    fn show(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
    ) -> (Option<ScriptureCitationAction>, bool) {
        let (chosen, open) = crate::menu::menu_at(
            ctx,
            Id::new("scripture-citation-menu"),
            self.pos,
            palette,
            std::mem::take(&mut self.just_opened),
            |ui| {
                if crate::menu::Item::new("Edit")
                    .icon(crate::icons::pencil)
                    .show(ui, palette)
                {
                    return Some(ScriptureCitationAction::Edit);
                }
                crate::menu::Item::new("Go to Scripture")
                    .icon(crate::icons::quote)
                    .show(ui, palette)
                    .then_some(ScriptureCitationAction::GoTo)
            },
        );
        let stays_open = open && chosen.is_none();
        (chosen, stays_open)
    }
}

enum ScriptureCitationAction {
    Edit,
    GoTo,
}

pub struct Editor {
    pub doc: Document,
    /// Namespaces widget ids so cursors don't leak between notes.
    note_id: String,
    pending_caret: Option<Caret>,
    /// Selection end (character index) paired with `pending_caret` when it is a range.
    pending_selection_end: Option<usize>,
    menu_selected: usize,
    scroll_menu: bool,
    /// A slash query the user closed with Escape: (block, byte offset of `/`).
    menu_dismissed: Option<(usize, usize)>,
    /// Last frame's layout of each block, for moving between blocks by row.
    galleys: HashMap<usize, Arc<Galley>>,
    /// Last frame's screen rect and text origin of each block, for mapping
    /// the pointer to a position in the document.
    layouts: HashMap<usize, (Rect, Pos2)>,
    /// A selection that spans blocks (or everything, after Cmd+A). Selections
    /// inside one block belong to that block's text field instead.
    selection: Option<Selection>,
    /// Where the current mouse press started, while the button is down.
    drag_anchor: Option<Caret>,
    /// Find in this note (Cmd+F).
    pub find: FindBar,
    /// Where the caret last moved to, and when, so it blinks like egui's.
    caret_moved: Option<((usize, usize), f64)>,
    meta: MetaEditor,
    /// The image block clicked last, which the keyboard acts on.
    selected_image: Option<usize>,
    /// The block that last had the caret, where pasted images go.
    last_focus: Option<usize>,
    /// A press on a link that should open it when it becomes a click.
    link_press: Option<LinkPress>,
    /// Last frame's caret (block, character), to tell which way it moved.
    last_caret: Option<(usize, usize)>,
    /// The "Add citation" form, while it's open.
    citation_form: Option<CitationForm>,
    /// The menu for a right-clicked link, while it's open.
    link_menu: Option<LinkMenu>,
    /// The menu for a right-clicked scripture citation, while it's open.
    scripture_citation_menu: Option<ScriptureCitationMenu>,
    /// The menu for a right-clicked misspelling, while it's open.
    spell_menu: Option<SpellMenu>,
    /// Highlight and underline colors for the right-clicked text.
    mark_menu: Option<MarkMenu>,
    /// Style and color the selection bar applies next. Remembered so the next
    /// selection opens on the mark you used last.
    bar_kind: MarkKind,
    bar_color: u32,
    /// The word the caret is still typing. `committed` means space, tab, or
    /// newline has finished it. Tab does not leave the word, so the line has
    /// to remember the key.
    spell_open: Option<SpellOpen>,
    /// The "Edit link" form, while it's open.
    link_form: Option<LinkForm>,
    /// Where the caret goes once the form has closed. egui keeps the form's
    /// modal layer for a frame, so focusing text any sooner doesn't stick.
    caret_after_form: Option<(Caret, u8)>,
    /// Downloads a page to fill in a citation (swapped out in tests).
    pub fetch_page: citation_form::Fetch,
    /// A citation to scroll to in the list, after its number was clicked.
    jump_to_citation: Option<String>,
    /// The list under the divider starts closed. A click on a number or on
    /// the section header opens it.
    citations_open: bool,
    /// The citation in the list that was jumped to, highlighted.
    highlighted_citation: Option<String>,
    /// Where each citation's first number is in the text, for jumping back.
    ref_rects: HashMap<String, Rect>,
    /// The scripture picker opened by `/scripture`.
    scripture_picker: Option<ScripturePicker>,
    /// The document picker opened by `/file-link`.
    file_picker: Option<FilePicker>,
    move_picker: Option<MovePicker>,
    /// Where the slash menu was drawn, so the scripture picker opens there.
    menu_anchor: Pos2,
    /// Undo and redo for the whole note.
    history: History,
    /// This frame's change came from a command (formatting, a paste, …) and
    /// must be its own undo step rather than join the typing around it.
    separate_change: bool,
    /// Outline items (by block index) whose sub items are hidden.
    /// Indices are remapped when the note's blocks move.
    collapsed: HashSet<usize>,
    /// Blocks hidden this frame because an ancestor outline item is collapsed.
    hidden: Vec<bool>,
    /// Collapsed items last written to the folder's `.scripture-study` file.
    persisted_outline: Vec<CollapsedOutline>,
    /// A fold or unfold should be written on the next frame, without waiting
    /// for the note itself to be saved.
    outline_dirty: bool,
}

/// A word the caret is still in, and whether a closing key has finished it.
struct SpellOpen {
    block: usize,
    word: String,
    committed: bool,
}

impl Editor {
    pub fn new(note_id: &str, doc: Document) -> Self {
        let last = doc.blocks.len() - 1;
        let caret = Caret {
            block: last,
            char: doc.blocks[last].text.chars().count(),
        };
        Self {
            history: History::new(&doc),
            citation_form: None,
            link_menu: None,
            scripture_citation_menu: None,
            spell_menu: None,
            mark_menu: None,
            bar_kind: MarkKind::Highlight,
            bar_color: DEFAULT_HIGHLIGHT,
            spell_open: None,
            link_form: None,
            caret_after_form: None,
            fetch_page: citation_form::fetch_with_curl,
            jump_to_citation: None,
            citations_open: false,
            highlighted_citation: None,
            ref_rects: HashMap::new(),
            scripture_picker: None,
            file_picker: None,
            move_picker: None,
            menu_anchor: Pos2::ZERO,
            separate_change: false,
            doc,
            note_id: note_id.to_string(),
            pending_caret: Some(caret),
            pending_selection_end: None,
            menu_selected: 0,
            scroll_menu: false,
            menu_dismissed: None,
            galleys: HashMap::new(),
            layouts: HashMap::new(),
            selection: None,
            drag_anchor: None,
            find: FindBar::default(),
            caret_moved: None,
            meta: MetaEditor::default(),
            selected_image: None,
            last_focus: None,
            link_press: None,
            last_caret: None,
            collapsed: HashSet::new(),
            hidden: Vec::new(),
            persisted_outline: Vec::new(),
            outline_dirty: false,
        }
    }

    /// Folds the outline items recorded for this note.
    pub fn restore_collapsed(&mut self, saved: &[CollapsedOutline]) {
        let wanted: HashSet<(String, usize)> = saved
            .iter()
            .map(|item| (item.key.clone(), item.nth))
            .collect();
        let mut seen: HashMap<String, usize> = HashMap::new();
        self.collapsed.clear();
        for (i, block) in self.doc.blocks.iter().enumerate() {
            let key = outline_key(block);
            let nth = *seen.entry(key.clone()).or_insert(0);
            if wanted.contains(&(key, nth)) && self.doc.outline_child_range(i).is_some() {
                self.collapsed.insert(i);
            }
            *seen.get_mut(&outline_key(block)).unwrap() += 1;
        }
        self.recompute_hidden();
        self.persisted_outline = saved.to_vec();
        // Drop entries that no longer match a parent item.
        self.outline_dirty = self.collapsed_outline() != saved;
    }

    /// Folded items, in document order, when they differ from what's saved.
    pub fn outline_changes(&self) -> Option<Vec<CollapsedOutline>> {
        let current = self.collapsed_outline();
        (current != self.persisted_outline).then_some(current)
    }

    /// True when a fold changed and should be written before the autosave delay.
    pub fn outline_needs_flush(&self) -> bool {
        self.outline_dirty && self.outline_changes().is_some()
    }

    pub fn mark_outline_persisted(&mut self) {
        self.persisted_outline = self.collapsed_outline();
        self.outline_dirty = false;
    }

    fn collapsed_outline(&self) -> Vec<CollapsedOutline> {
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut items = Vec::new();
        for (i, block) in self.doc.blocks.iter().enumerate() {
            let key = outline_key(block);
            let nth = *seen.entry(key.clone()).or_insert(0);
            if self.collapsed.contains(&i) && self.doc.outline_child_range(i).is_some() {
                items.push(CollapsedOutline {
                    key: key.clone(),
                    nth,
                });
            }
            *seen.get_mut(&key).unwrap() += 1;
        }
        items
    }

    /// Leaves the keyboard focus where it is instead of placing the caret
    /// when the note opened (e.g. a sidebar field is being typed in).
    pub fn skip_initial_focus(&mut self) {
        self.pending_caret = None;
        self.pending_selection_end = None;
    }

    /// Replaces the note's content from outside the editor (e.g. a rename),
    /// as one undoable step.
    pub fn replace_doc(&mut self, doc: Document) {
        self.doc = doc;
        self.doc.ensure_not_empty();
        self.galleys.clear();
        self.layouts.clear();
        self.selection = None;
        self.selected_image = None;
        self.separate_change = true;
    }

    /// Adds an image after the block being edited (replacing it if it's an
    /// empty paragraph), with an empty paragraph below to keep typing in.
    pub fn insert_image(&mut self, src: String) {
        let outline_before = self.outline_keys();
        let doc = &mut self.doc;
        let last = doc.blocks.len() - 1;
        let at = self
            .selected_image
            .or(self.last_focus)
            .unwrap_or(last)
            .min(last);
        let image = Block::new(
            BlockKind::Image {
                src,
                alt: String::new(),
            },
            "",
        );
        // The picture lines up with the line it's added at.
        let block = &doc.blocks[at];
        let indent = match block.kind {
            BlockKind::Paragraph => {
                let last_line = block.text.rsplit('\n').next().unwrap_or("");
                ops::leading_tabs(last_line)
                    .min(scripture_study_core::document::MAX_INDENT as usize) as u8
            }
            _ => block.indent,
        };
        let image = image.indented(indent);
        let empty = |b: &Block| b.kind == BlockKind::Paragraph && b.text.trim().is_empty();
        let index = if empty(&doc.blocks[at]) {
            doc.blocks[at] = image;
            at
        } else {
            doc.blocks.insert(at + 1, image);
            at + 1
        };
        if !doc.blocks.get(index + 1).is_some_and(empty) {
            let tabs = "\t".repeat(indent as usize);
            doc.blocks.insert(index + 1, Block::paragraph(tabs));
        }
        self.selected_image = None;
        self.selection = None;
        self.separate_change = true;
        self.galleys.clear();
        self.layouts.clear();
        self.pending_selection_end = None;
        let char = ops::leading_tabs(&doc.blocks[index + 1].text);
        self.rebind_outline(&outline_before);
        self.pending_caret = Some(Caret {
            block: index + 1,
            char,
        });
    }

    /// Puts the caret at a place in the note.
    #[cfg(test)]
    pub fn place_caret(&mut self, block: usize, char: usize) {
        self.pending_caret = Some(Caret { block, char });
    }

    /// Where citation `id`'s number is drawn in the text.
    #[cfg(test)]
    pub fn citation_number_rect(&self, id: &str) -> Option<Rect> {
        self.ref_rects.get(id).copied()
    }

    #[cfg(test)]
    pub fn highlighted_citation(&self) -> Option<&str> {
        self.highlighted_citation.as_deref()
    }

    #[cfg(test)]
    pub fn citation_form(&mut self) -> Option<&mut CitationForm> {
        self.citation_form.as_mut()
    }

    #[cfg(test)]
    pub fn selected_image(&self) -> Option<usize> {
        self.selected_image
    }

    #[cfg(test)]
    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    fn block_id(&self, index: usize) -> Id {
        Id::new(("block", &self.note_id, index))
    }

    /// Misspelled words that are ready for a red line. The word under the
    /// caret stays bare until space, tab, or newline finishes it.
    fn spell_ranges(
        &mut self,
        spelling: &mut Spelling,
        block: usize,
        caret_byte: Option<usize>,
        boundary: bool,
    ) -> Vec<Range<usize>> {
        let typing = caret_byte.and_then(|caret| {
            let text = &self.doc.blocks[block].text;
            spell::open_word(text, caret).map(|range| text[range].to_string())
        });
        if let Some(word) = typing {
            match &mut self.spell_open {
                Some(open) if open.block == block && open.word == word => {
                    if boundary {
                        open.committed = true;
                    }
                }
                _ => {
                    self.spell_open = Some(SpellOpen {
                        block,
                        word,
                        committed: boundary,
                    });
                }
            }
        }
        let committed = self
            .spell_open
            .as_ref()
            .filter(|open| open.committed && open.block == block)
            .map(|open| open.word.clone());
        let text = &self.doc.blocks[block].text;
        spelling
            .misspellings(text)
            .into_iter()
            .filter(|range| {
                let word_committed = committed.as_deref() == Some(&text[range.clone()]);
                spell::ready_to_mark(text, range, caret_byte, word_committed)
            })
            .collect()
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        commands: &[Command],
        notes: &[NoteMeta],
        times: meta::NoteTimes,
        dir: &Path,
        spelling: &mut Spelling,
    ) -> Vec<Event> {
        spelling.set_repaint(ui.ctx());
        // Read before later code consumes the keys. Space lands in the text
        // during the field; Tab and Enter are applied after this frame draws.
        let spell_boundary = ui.input(|input| {
            input.key_pressed(Key::Space)
                || input.key_pressed(Key::Tab)
                || input.key_pressed(Key::Enter)
        });
        let palette = Palette::for_ui(ui);
        let mut events = Vec::new();
        if self.undo_keys(ui) {
            events.push(Event::Changed);
        }
        // A paste or cut is a step of its own, even inside one block.
        if ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::Cut))
        }) {
            self.separate_change = true;
        }
        if self
            .meta
            .show(ui, &mut self.doc.properties, times, &palette, &self.note_id)
        {
            events.push(Event::Changed);
        }
        self.find.update(&self.doc);
        // A new press forgets a link pressed earlier; the block under the
        // pointer records it again if it's on a link.
        if ui.input(|i| i.pointer.primary_pressed()) {
            self.link_press = None;
            self.highlighted_citation = None;
        }
        // A picker search owns the keyboard while it's open.
        if self.scripture_picker.is_some()
            || self.file_picker.is_some()
            || self.move_picker.is_some()
        {
            self.pending_caret = None;
            self.pending_selection_end = None;
            for i in 0..self.doc.blocks.len() {
                let id = self.block_id(i);
                ui.memory_mut(|m| m.surrender_focus(id));
            }
        }
        self.ref_rects.clear();
        if let Some((caret, frames)) = self.caret_after_form.take() {
            if frames == 0 {
                self.pending_caret = Some(caret);
            } else {
                self.caret_after_form = Some((caret, frames - 1));
                ui.ctx().request_repaint();
            }
        }
        let pending = self.pending_caret.take();
        let pending_end = self.pending_selection_end.take();
        // A click that opened this note (e.g. in the sidebar) would steal
        // the focus right back, so place the caret again next frame.
        if pending.is_some() && ui.input(|i| i.pointer.any_click()) {
            self.pending_caret = pending;
            self.pending_selection_end = pending_end;
        }
        let mut op = self.selection_input(ui);
        let picker_open = self.scripture_picker.is_some()
            || self.file_picker.is_some()
            || self.move_picker.is_some();
        if self.selection.is_some() && !picker_open {
            op = op.or_else(|| self.selection_keys(ui));
            // The document selection owns the keyboard; no block keeps a caret.
            for i in 0..self.doc.blocks.len() {
                let id = self.block_id(i);
                if ui.memory(|m| m.has_focus(id)) {
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
            }
        }
        if op.is_none() {
            op = self.image_keys(ui);
        }
        let selection_color = ui.visuals().selection.bg_fill;
        let mut menu: Option<OpenMenu> = None;
        // Where the selection bar anchors: the first line of selected text.
        let mut bar_target: Option<(Rect, bool, usize, usize, usize)> = None;
        let numbers = self.doc.paragraph_numbers();
        // A caret or a find jump inside a folded item opens it, so the
        // place we're going is on screen. Folding itself never leaves the
        // caret in a hidden block.
        if self.find.reveal {
            if let Some(m) = self.find.current_match() {
                self.reveal_outline(m.block);
            }
        }
        if let Some(id) = self.jump_to_citation.clone() {
            if let Some(block) = self
                .doc
                .blocks
                .iter()
                .position(|b| citations::refs(&b.text).iter().any(|r| r.id == id))
            {
                self.reveal_outline(block);
            }
        }
        self.recompute_hidden();
        if let Some(caret) = pending {
            if self.hidden.get(caret.block).copied().unwrap_or(false) {
                self.reveal_outline(caret.block);
                self.recompute_hidden();
            }
        }
        // Applied after the loop so splitting a line into blocks doesn't
        // shift the ones still being drawn.
        let mut markdown_shortcut = None;

        for i in 0..self.doc.blocks.len() {
            if self.hidden.get(i).copied().unwrap_or(false) {
                let id = self.block_id(i);
                if ui.memory(|m| m.has_focus(id)) {
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
                continue;
            }
            let id = self.block_id(i);
            let kind = self.doc.blocks[i].kind.clone();
            ui.add_space(match kind {
                BlockKind::Heading(1) if i > 0 => 22.0,
                BlockKind::Heading(_) if i > 0 => 14.0,
                _ => 3.0,
            });

            let indent = self.doc.blocks[i].indent as f32 * INDENT_WIDTH;
            if let BlockKind::Image { src, alt } = &kind {
                let path = dir.join(src);
                if let Some(image_op) = self.image_block(ui, i, &path, alt, indent, &palette) {
                    op = Some(image_op);
                }
                continue;
            }

            if kind == BlockKind::Divider {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                let rect = rect.with_min_x(rect.min.x + indent);
                self.layouts.insert(i, (rect, rect.left_top()));
                self.galleys.remove(&i);
                if self
                    .selection
                    .and_then(|s| s.in_block(&self.doc, i))
                    .is_some()
                {
                    ui.painter().rect_filled(rect, 3.0, selection_color);
                }
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0, palette.border),
                );
                continue;
            }

            if let Some(caret) = pending.filter(|c| c.block == i) {
                let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                let end = pending_end.unwrap_or(caret.char);
                state.cursor.set_char_range(Some(CCursorRange::two(
                    CCursor::new(caret.char.min(end)),
                    CCursor::new(caret.char.max(end)),
                )));
                TextEdit::store_state(ui.ctx(), id, state);
                ui.memory_mut(|m| m.request_focus(id));
            }
            let focused = ui.memory(|m| m.has_focus(id));
            if focused {
                self.last_focus = Some(i);
            }
            let selection = focused
                .then(|| TextEdit::load_state(ui.ctx(), id))
                .flatten()
                .and_then(|s| s.cursor.char_range())
                .map(|r| {
                    let r = r.as_sorted_char_range();
                    (r.start.0, r.end.0)
                });

            // Slash menu for this block?
            let text = &self.doc.blocks[i].text;
            let slash = selection
                .filter(|(start, end)| start == end && !matches!(kind, BlockKind::Code { .. }))
                .and_then(|(caret, _)| ops::slash_query(text, caret));
            if self.menu_dismissed.is_some_and(|(b, start)| {
                b == i && focused && slash.as_ref().is_none_or(|q| q.start != start)
            }) {
                self.menu_dismissed = None;
            }
            let slash = slash.filter(|q| self.menu_dismissed != Some((i, q.start)));
            let matches = slash
                .as_ref()
                .map(|q| commands::filter(commands, &q.query))
                .filter(|m| !m.is_empty());

            if let (Some((start, end)), None) = (selection, &op) {
                op = match (&slash, &matches) {
                    (Some(query), Some(matches)) => self.menu_keys(ui, i, query, matches),
                    _ => self.editing_keys(ui, i, start, end),
                };
            }

            // Draw the block, reserving a spot behind it for the selection.
            let highlight = ui.painter().add(Shape::Noop);
            let interactive = self.selection.is_none();
            let muted = kind == (BlockKind::Todo { checked: true });
            let caret_stroke = ui.visuals().text_cursor.stroke;
            let row = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // egui's caret spans the whole line slot, spacing included;
                // `paint_caret` draws one as tall as the text instead.
                ui.visuals_mut().text_cursor.stroke.color = Color32::TRANSPARENT;
                ui.add_space(indent);
                if let Some(toggle) = self.gutter(ui, i, &kind, &palette) {
                    op = Some(toggle);
                }
                // No text yet (tags or other properties don't count).
                let empty_note = self.doc.blocks.len() == 1
                    && self.doc.blocks[0].kind == BlockKind::Paragraph
                    && self.doc.blocks[0].text.is_empty();
                let block = &mut self.doc.blocks[i];
                let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, wrap: f32| {
                    theme::layout(ui, buf.as_str(), &kind, muted, focused, wrap)
                };
                let edit = TextEdit::multiline(&mut block.text)
                    .id(id)
                    // Tab indents the block rather than moving to another field.
                    .lock_focus(true)
                    .interactive(interactive)
                    .frame(Frame::NONE)
                    .margin(Margin::ZERO)
                    .desired_rows(1)
                    .desired_width(ui.available_width())
                    .hint_text(placeholder(&kind, focused, empty_note, &palette))
                    .layouter(&mut layouter)
                    .return_key(match kind {
                        BlockKind::Code { .. } => {
                            KeyboardShortcut::new(Modifiers::NONE, Key::Enter)
                        }
                        _ => KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter),
                    });
                if let BlockKind::Code { .. } = kind {
                    Frame::new()
                        .fill(palette.code_bg)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(16, 14))
                        .show(ui, |ui| edit.show(ui))
                        .inner
                } else {
                    edit.show(ui)
                }
            });
            let output = row.inner;
            let ranges = if matches!(kind, BlockKind::Code { .. }) {
                Vec::new()
            } else {
                let caret_byte = if focused {
                    let text = &self.doc.blocks[i].text;
                    TextEdit::load_state(ui.ctx(), id)
                        .and_then(|state| state.cursor.char_range())
                        .or(output.cursor_range)
                        .map(|range| ops::char_to_byte(text, range.primary.index.0))
                } else {
                    None
                };
                self.spell_ranges(spelling, i, caret_byte, spell_boundary)
            };
            if !ranges.is_empty() {
                spell::paint_squiggles(
                    ui.painter(),
                    &output.galley,
                    output.galley_pos,
                    &self.doc.blocks[i].text,
                    &ranges,
                    theme::text_height(ui, &kind),
                );
            }
            self.layouts
                .insert(i, (row.response.rect, output.galley_pos));
            let mut shapes = Vec::new();
            if self.find.open {
                let (all, current_color) = find_bar::match_colors(ui);
                let current = self.find.current_match();
                for m in self.find.matches.iter().filter(|m| m.block == i) {
                    let color = if Some(*m) == current {
                        current_color
                    } else {
                        all
                    };
                    shapes.extend(selection_rects(
                        &output.galley,
                        output.galley_pos,
                        m.start,
                        m.end,
                        false,
                        color,
                    ));
                }
                if let Some(m) = current.filter(|m| m.block == i && self.find.reveal) {
                    let rect = find_bar::match_rect(&output.galley, output.galley_pos, m.start);
                    ui.scroll_to_rect(rect.expand(48.0), Some(egui::Align::Center));
                    self.find.reveal = false;
                }
            }
            if let Some((from, to, continues)) =
                self.selection.and_then(|s| s.in_block(&self.doc, i))
            {
                shapes.extend(selection_rects(
                    &output.galley,
                    output.galley_pos,
                    from,
                    to,
                    continues,
                    selection_color,
                ));
            }
            if !shapes.is_empty() {
                ui.painter().set(highlight, Shape::Vec(shapes));
            }
            if bar_target.is_none() && kind.has_text() && !matches!(kind, BlockKind::Code { .. }) {
                let field = selection.filter(|(start, end)| start != end);
                let across = self
                    .selection
                    .filter(|sel| !sel.is_empty())
                    .and_then(|sel| sel.in_block(&self.doc, i))
                    .filter(|(from, to, _)| from != to);
                let range = field
                    .map(|(start, end)| (false, start, end))
                    .or_else(|| across.map(|(from, to, _)| (true, from, to)));
                if let Some((across, start, end)) = range {
                    if let Some(rect) = span_rect(&output.galley, output.galley_pos, start, end) {
                        bar_target = Some((rect, across, i, start, end));
                    }
                }
            }

            if focused {
                self.skip_link_markup(ui, i, id);
            }
            // After skipping link markup, so the caret is drawn where the
            // cursor ended up rather than inside hidden syntax.
            if interactive && output.response.has_focus() {
                let cursor = TextEdit::load_state(ui.ctx(), id)
                    .and_then(|state| state.cursor.char_range())
                    .or(output.cursor_range)
                    .map(|range| range.primary);
                if let Some(cursor) = cursor {
                    self.paint_caret(ui, i, &kind, &output, cursor, caret_stroke);
                }
            }

            // Right-clicking a misspelled word opens its menu. A link that
            // isn't misspelled keeps the link menu.
            if output.response.secondary_clicked() {
                let hit = ui
                    .input(|input| input.pointer.interact_pos())
                    .and_then(|p| {
                        let text = &self.doc.blocks[i].text;
                        let byte = char_under(&output.galley, p - output.galley_pos, text)?;
                        let word = ranges.iter().find(|range| range.contains(&byte)).cloned();
                        let word = word.map(|range| {
                            let word = text[range.clone()].to_string();
                            (range, word)
                        });
                        let citation = citations::ref_at(text, ops::byte_to_char(text, byte))
                            .filter(|id| {
                                self.doc.citations.iter().any(|c| &c.id == id)
                                    && scriptures::find_verses(id)
                                        .iter()
                                        .any(|hit| hit.label == *id)
                            });
                        let link = LinkTarget::at(i, text, byte);
                        Some((p, byte, word, link, citation))
                    });
                if let Some((pos, byte, word, link, citation)) = hit {
                    let doc_sel = self.selection.filter(|s| !s.is_empty());
                    let field = selection.filter(|(start, end)| start != end);
                    // A selection is what gets annotated. Otherwise a misspelling
                    // or a link keeps its own menu, which also offers marks.
                    if doc_sel.is_some() || field.is_some() {
                        let (start, end) = field.unwrap_or((0, 0));
                        self.mark_menu = Some(MarkMenu::new(doc_sel.is_some(), i, start, end, pos));
                        self.spell_menu = None;
                        self.link_menu = None;
                    } else if let Some(reference) = citation {
                        self.scripture_citation_menu =
                            Some(ScriptureCitationMenu::new(reference, pos));
                        self.spell_menu = None;
                        self.link_menu = None;
                    } else if let Some((range, word)) = word {
                        let suggestions = spelling.suggestions(&word);
                        self.spell_menu =
                            Some(SpellMenu::new(i, range, word, suggestions, pos, link));
                        self.link_menu = None;
                        self.mark_menu = None;
                    } else if let Some(target) = link {
                        self.link_menu = Some(LinkMenu::new(target, pos));
                        self.spell_menu = None;
                        self.mark_menu = None;
                    } else {
                        let char = ops::byte_to_char(&self.doc.blocks[i].text, byte);
                        self.mark_menu = Some(MarkMenu::new(false, i, char, char, pos));
                        self.spell_menu = None;
                        self.link_menu = None;
                    }
                }
            }

            if let Some(press) = self.link_click(ui, i, &output, focused, notes) {
                match press.target {
                    Target::Url(url) => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
                    Target::Note(id) => events.push(Event::OpenNote(id)),
                    Target::Citation(id) => self.jump_to_citation = Some(id),
                }
                if !press.editing {
                    // Opening a link isn't a reason to start editing the block.
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
            }

            // Remember where citation numbers are, to jump back to them.
            if !matches!(kind, BlockKind::Code { .. }) {
                let text = &self.doc.blocks[i].text;
                for r in citations::refs(text) {
                    let char = ops::byte_to_char(text, r.label().start);
                    let at = output.galley.pos_from_cursor(CCursor::new(char));
                    self.ref_rects
                        .entry(r.id)
                        .or_insert(at.translate(output.galley_pos.to_vec2()));
                }
            }

            if let Some(n) = numbers.get(i).copied().flatten() {
                // The paragraph's number, dimmed, in the margin by its first line.
                // An indented paragraph's number sits with that line.
                let first = self.doc.blocks[i].text.split('\n').next().unwrap_or("");
                let line_indent = if kind == BlockKind::Paragraph {
                    ops::leading_tabs(first) as f32 * INDENT_WIDTH
                } else {
                    0.0
                };
                let first_line = output.galley.pos_from_cursor(CCursor::new(0));
                self.paragraph_number(
                    ui,
                    i,
                    n,
                    output.galley_pos.x + line_indent - indent - PARAGRAPH_NUMBER_GAP,
                    output.galley_pos.y + first_line.center().y,
                    &palette,
                );
            }

            if kind == BlockKind::Quote {
                let rect = output.response.rect;
                let x = rect.left() - QUOTE_INDENT + 1.5;
                ui.painter()
                    .vline(x, rect.y_range(), Stroke::new(3.0, palette.text));
            }

            if output.response.changed() {
                events.push(Event::Changed);
                self.menu_selected = 0;
                let caret = output
                    .cursor_range
                    .map(|range| range.primary.index.0)
                    .unwrap_or_else(|| selection.map_or(0, |(start, _)| start + 1));
                markdown_shortcut = Some((i, caret));
            }

            if let (Some(query), Some(matches)) = (slash, matches) {
                let slash_pos = output
                    .galley
                    .pos_from_cursor(CCursor::new(ops::byte_to_char(
                        &self.doc.blocks[i].text,
                        query.start,
                    )));
                let anchor = output.galley_pos + slash_pos.left_bottom().to_vec2() + vec2(0.0, 6.0);
                self.menu_anchor = anchor;
                menu = Some(OpenMenu {
                    block: i,
                    query,
                    matches,
                    anchor,
                });
            }
            self.galleys.insert(i, output.galley);
        }

        if !self.doc.citations.is_empty() {
            self.citation_list(ui, &palette);
        }
        // Edits made from a menu or form place the caret once it has closed.
        let mut from_form = false;
        let link_colors = self.link_menu.as_ref().and_then(|menu| {
            let text = &self.doc.blocks.get(menu.target.block)?.text;
            if !menu.target.still_in(text) {
                return None;
            }
            let start = ops::byte_to_char(text, menu.target.label.start);
            let end = ops::byte_to_char(text, menu.target.label.end);
            Some(mark_colors(text, start, end))
        });
        if let Some(link_menu) = &mut self.link_menu {
            let (highlight, underline) = link_colors.unwrap_or((None, None));
            let (choice, open) = link_menu.show(ui.ctx(), &palette, highlight, underline);
            let target = link_menu.target.clone();
            if !open {
                self.link_menu = None;
            }
            if let Some(choice) = choice {
                if let Some(next) = self.apply_link_choice(choice, target, ui) {
                    op = Some(next);
                }
            }
        }
        if let Some(mut citation_menu) = self.scripture_citation_menu.take() {
            let reference = citation_menu.reference.clone();
            let position = citation_menu.pos;
            let (chosen, open) = citation_menu.show(ui.ctx(), &palette);
            if open {
                self.scripture_citation_menu = Some(citation_menu);
            }
            match chosen {
                Some(ScriptureCitationAction::Edit) => {
                    self.scripture_picker = Some(ScripturePicker::for_edit(reference, position));
                }
                Some(ScriptureCitationAction::GoTo) => {
                    events.push(Event::OpenScripture(reference));
                }
                None => {}
            }
        }
        let spell_still = self
            .spell_menu
            .as_ref()
            .map(|menu| (menu.block, menu.range.clone(), menu.word.clone()));
        if let Some((block, range, word)) = spell_still {
            let valid = self
                .doc
                .blocks
                .get(block)
                .and_then(|block| block.text.get(range))
                == Some(word.as_str());
            if !valid {
                self.spell_menu = None;
            }
        }
        let spell_colors = self.spell_menu.as_ref().and_then(|menu| {
            let text = self.doc.blocks.get(menu.block)?.text.as_str();
            let start = ops::byte_to_char(text, menu.range.start.min(text.len()));
            let end = ops::byte_to_char(text, menu.range.end.min(text.len()));
            Some(mark_colors(text, start, end))
        });
        let spell_choice = self.spell_menu.as_mut().map(|menu| {
            let (highlight, underline) = spell_colors.unwrap_or((None, None));
            let (choice, open) = menu.show(ui.ctx(), &palette, highlight, underline);
            (
                choice,
                open,
                menu.block,
                menu.range.clone(),
                menu.word.clone(),
                menu.link.clone(),
            )
        });
        if let Some((choice, open, block, range, word, link)) = spell_choice {
            if !open {
                self.spell_menu = None;
            }
            match choice {
                Some(SpellChoice::Replace(with)) => {
                    if let Some(current) = self.doc.blocks.get(block).map(|b| b.text.clone()) {
                        if current.get(range.clone()) == Some(word.as_str()) {
                            let char =
                                ops::byte_to_char(&current, range.start) + with.chars().count();
                            let mut text = current;
                            text.replace_range(range, &with);
                            self.separate_change = true;
                            op = Some(Op::SetText {
                                block,
                                text,
                                char,
                                end: char,
                            });
                        }
                    }
                }
                Some(SpellChoice::IgnoreAll) => spelling.ignore_all(&word),
                Some(SpellChoice::AddToDictionary) => spelling.add(&word),
                Some(SpellChoice::Link(choice)) => {
                    if let Some(target) = link {
                        if let Some(next) = self.apply_link_choice(choice, target, ui) {
                            op = Some(next);
                        }
                    }
                }
                Some(SpellChoice::Mark(choice)) => {
                    if let Some(current) = self.doc.blocks.get(block).map(|b| b.text.clone()) {
                        if current.get(range.clone()) == Some(word.as_str()) {
                            let start = ops::byte_to_char(&current, range.start);
                            let end = ops::byte_to_char(&current, range.end);
                            op = Some(Op::Mark {
                                selection: false,
                                block,
                                start,
                                end,
                                kind: choice.kind,
                                color: choice.color,
                                toggle: choice.toggle,
                            });
                        }
                    }
                }
                None => {}
            }
        }
        let shown_colors = self.mark_menu.as_ref().map(|menu| {
            if menu.selection {
                self.selection
                    .map(|sel| {
                        (
                            marks::shared_color(&self.doc, &sel, MarkKind::Highlight),
                            marks::shared_color(&self.doc, &sel, MarkKind::Underline),
                        )
                    })
                    .unwrap_or((None, None))
            } else {
                self.doc
                    .blocks
                    .get(menu.block)
                    .map(|block| mark_colors(&block.text, menu.start, menu.end))
                    .unwrap_or((None, None))
            }
        });
        let mark_choice =
            self.mark_menu
                .as_mut()
                .zip(shown_colors)
                .map(|(menu, (highlight, underline))| {
                    let (choice, open) = menu.show(ui.ctx(), &palette, highlight, underline);
                    (
                        choice,
                        open,
                        menu.selection,
                        menu.block,
                        menu.start,
                        menu.end,
                    )
                });
        if let Some((choice, open, selection, block, start, end)) = mark_choice {
            if !open {
                self.mark_menu = None;
            }
            if let Some(choice) = choice {
                op = Some(Op::Mark {
                    selection,
                    block,
                    start,
                    end,
                    kind: choice.kind,
                    color: choice.color,
                    toggle: choice.toggle,
                });
            }
        }
        // The bar stays up for a finished selection. It hides while the pointer
        // is still dragging, so it doesn't sit under the cursor.
        if bar_target.is_some() && ui.input(|i| i.pointer.is_decidedly_dragging()) {
            bar_target = None;
        }
        if let Some((rect, selection, block, start, end)) = bar_target {
            let (highlight, underline) = if selection {
                self.selection
                    .map(|sel| {
                        (
                            marks::shared_color(&self.doc, &sel, MarkKind::Highlight),
                            marks::shared_color(&self.doc, &sel, MarkKind::Underline),
                        )
                    })
                    .unwrap_or((None, None))
            } else {
                self.doc
                    .blocks
                    .get(block)
                    .map(|block| mark_colors(&block.text, start, end))
                    .unwrap_or((None, None))
            };
            match (self.bar_kind, highlight, underline) {
                (MarkKind::Underline, _, Some(color)) => self.bar_color = color,
                (_, Some(color), _) => {
                    self.bar_kind = MarkKind::Highlight;
                    self.bar_color = color;
                }
                (_, None, Some(color)) => {
                    self.bar_kind = MarkKind::Underline;
                    self.bar_color = color;
                }
                _ => {}
            }
            if let Some(action) =
                crate::marks::show_bar(ui.ctx(), &palette, rect, self.bar_kind, self.bar_color)
            {
                match action {
                    BarAction::Apply { kind, color } => {
                        self.bar_kind = kind;
                        self.bar_color = color;
                        op = Some(Op::Mark {
                            selection,
                            block,
                            start,
                            end,
                            kind,
                            color: Some(color),
                            toggle: false,
                        });
                    }
                    BarAction::Clear => {
                        op = Some(Op::ClearMarks {
                            selection,
                            block,
                            start,
                            end,
                        });
                    }
                    BarAction::Copy => {
                        if let Some(sel) = self.selection {
                            ui.ctx().copy_text(selection::to_markdown(&self.doc, &sel));
                        }
                    }
                    BarAction::MoveTo => {
                        // Clicking the bar can return focus to the text field,
                        // which clears the document-level selection. Rebuild
                        // it from the range the bar was drawn for so the
                        // picker still has the text to move.
                        if !selection {
                            self.selection = Some(Selection::new(
                                Caret { block, char: start },
                                Caret { block, char: end },
                            ));
                        }
                        self.move_picker = Some(MovePicker::new(Pos2::new(
                            rect.left(),
                            rect.bottom() + 12.0,
                        )));
                    }
                }
            }
        }
        if let Some(form) = &mut self.link_form {
            match form.show(ui.ctx(), &palette) {
                link_menu::FormOutcome::Open => {}
                link_menu::FormOutcome::Cancel => self.link_form = None,
                link_menu::FormOutcome::Save(markdown) => {
                    let target = form.target.clone();
                    self.link_form = None;
                    if let Some(block) = self.doc.blocks.get(target.block) {
                        if target.still_in(&block.text) {
                            let (text, char) = target.replace(&block.text, &markdown);
                            from_form = true;
                            op = Some(Op::SetText {
                                block: target.block,
                                text,
                                char,
                                end: char,
                            });
                        }
                    }
                }
            }
        }
        if let Some(form) = &mut self.citation_form {
            match form.show(ui.ctx(), &palette, self.fetch_page) {
                Outcome::Open => {}
                Outcome::Cancel => {
                    let caret = Caret {
                        block: form.block,
                        char: form.char,
                    };
                    self.caret_after_form = Some((caret, 1));
                    self.citation_form = None;
                }
                Outcome::Insert(text) => {
                    op = Some(match form.converting.take() {
                        Some(target) => Op::ConvertLinkToCitation { target, text },
                        None => Op::InsertCitation {
                            block: form.block,
                            char: form.char,
                            text,
                        },
                    });
                    self.citation_form = None;
                }
            }
        }

        // Clicking the empty page below the last block starts typing there.
        let rest = ui.available_rect_before_wrap();
        let rest = rest.with_max_y(rest.max.y.max(rest.min.y + 200.0));
        // A menu choice this frame wins. The click that picked it can also land
        // on the empty page, which would otherwise replace the choice.
        if op.is_none() && ui.allocate_rect(rest, Sense::click()).clicked() {
            op = Some(Op::FocusEnd);
        }

        if let Some(menu) = menu {
            if let Some(action) = self.show_menu(ui, &menu, &palette) {
                op = Some(Op::Run {
                    block: menu.block,
                    query: menu.query,
                    action,
                });
            }
        }

        if op.is_none() {
            if let Some((index, caret)) = markdown_shortcut {
                let outline_before = self.outline_keys();
                if let Some(caret) = ops::apply_markdown_shortcut(&mut self.doc, index, caret) {
                    self.rebind_outline(&outline_before);
                    op = Some(Op::Focus(caret));
                }
            }
        }

        if let Some(op) = op {
            if let Some(event) = self.apply(op) {
                events.push(event);
            }
            if from_form {
                if let Some(caret) = self.pending_caret.take() {
                    self.caret_after_form = Some((caret, 1));
                }
            }
        }
        if let Some(picker) = self.scripture_picker.as_mut() {
            let outcome = picker.show(ui, &self.note_id, &palette);
            match outcome {
                crate::scripture_menu::Outcome::Open => {}
                crate::scripture_menu::Outcome::Cancel => {
                    let picker = self.scripture_picker.take().expect("picker");
                    self.pending_caret = Some(Caret {
                        block: picker.block,
                        char: picker.at,
                    });
                }
                crate::scripture_menu::Outcome::Insert(hit) => {
                    let picker = self.scripture_picker.take().expect("picker");
                    let op = match picker.editing {
                        Some(old_label) => Op::EditScripture {
                            old_label,
                            new_label: hit.label,
                            text: hit.text,
                        },
                        None => Op::InsertScripture {
                            block: picker.block,
                            at: picker.at,
                            label: hit.label,
                            text: hit.text,
                        },
                    };
                    if let Some(event) = self.apply(op) {
                        events.push(event);
                    }
                    if let Some(caret) = self.pending_caret.take() {
                        self.caret_after_form = Some((caret, 1));
                    }
                }
            }
        }
        if let Some(picker) = self.file_picker.as_mut() {
            let outcome = picker.show(ui, &self.note_id, notes, &palette);
            match outcome {
                crate::file_menu::Outcome::Open => {}
                crate::file_menu::Outcome::Cancel => {
                    let picker = self.file_picker.take().expect("picker");
                    self.pending_caret = Some(Caret {
                        block: picker.block,
                        char: picker.at,
                    });
                }
                crate::file_menu::Outcome::Insert(choice) => {
                    let picker = self.file_picker.take().expect("picker");
                    if let Some(event) = self.apply(Op::InsertNoteLink {
                        block: picker.block,
                        at: picker.at,
                        id: choice.id,
                        title: choice.title,
                    }) {
                        events.push(event);
                    }
                    if let Some(caret) = self.pending_caret.take() {
                        self.caret_after_form = Some((caret, 1));
                    }
                }
                crate::file_menu::Outcome::Move(_) => unreachable!("link picker cannot move"),
            }
        }
        if let Some(picker) = &mut self.move_picker {
            match picker.show(ui, &self.note_id, notes, &palette) {
                crate::file_menu::Outcome::Open => {}
                crate::file_menu::Outcome::Cancel => self.move_picker = None,
                crate::file_menu::Outcome::Move(destination) => {
                    self.move_picker = None;
                    if self.selection.is_some() {
                        if let Some(event) = self.apply(Op::MoveSelection {
                            destination: destination.id,
                        }) {
                            events.push(event);
                        }
                    }
                }
                crate::file_menu::Outcome::Insert(_) => {
                    unreachable!("move picker inserts no links")
                }
            }
        }
        if events.iter().any(|e| matches!(e, Event::Changed)) {
            // Keep citation numbers in text order after any edit (a number
            // deleted, text with citations pasted or moved…).
            citations::renumber(&mut self.doc);
        }
        let now = ui.input(|i| i.time);
        let caret = self.caret(ui);
        let separate = std::mem::take(&mut self.separate_change);
        self.history.record(&self.doc, caret, now, separate);
        events
    }

    /// Cmd+Z undoes; Cmd+Shift+Z or Cmd+Y redoes. Taken before the text
    /// fields see them, since each field's own undo only knows its own text.
    /// Returns whether the document changed.
    fn undo_keys(&mut self, ui: &mut Ui) -> bool {
        // Leave the keys to other text fields (a sidebar rename, the find bar).
        let focused = ui.memory(|m| m.focused());
        let ours =
            focused.is_none_or(|f| (0..self.doc.blocks.len()).any(|i| self.block_id(i) == f));
        if !ours {
            return false;
        }
        let redo = consume_mods(ui, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || consume_mods(ui, Modifiers::COMMAND, Key::Y);
        let undo = !redo && consume_mods(ui, Modifiers::COMMAND, Key::Z);
        let caret = self.caret(ui);
        let snapshot = match (undo, redo) {
            (true, _) => self.history.undo(&self.doc, caret),
            (_, true) => self.history.redo(&self.doc, caret),
            _ => None,
        };
        let Some(snapshot) = snapshot else {
            return false;
        };
        let outline_before = self.outline_keys();
        self.doc = snapshot.doc;
        self.doc.ensure_not_empty();
        self.rebind_outline(&outline_before);
        self.galleys.clear();
        self.layouts.clear();
        self.selection = None;
        self.selected_image = None;
        self.menu_dismissed = None;
        self.pending_selection_end = None;
        let last = self.doc.blocks.len() - 1;
        let end = |b: usize| self.doc.blocks[b].text.chars().count();
        self.pending_caret = Some(match snapshot.caret {
            Some(c) if c.block <= last => Caret {
                block: c.block,
                char: c.char.min(end(c.block)),
            },
            _ => Caret {
                block: last,
                char: end(last),
            },
        });
        true
    }

    /// Where the caret is, if a block has it.
    fn caret(&self, ui: &Ui) -> Option<Caret> {
        (0..self.doc.blocks.len()).find_map(|i| {
            let id = self.block_id(i);
            if !ui.memory(|m| m.has_focus(id)) {
                return None;
            }
            let range = TextEdit::load_state(ui.ctx(), id)?.cursor.char_range()?;
            Some(Caret {
                block: i,
                char: range.primary.index.0,
            })
        })
    }

    /// Keys while the slash menu is open. Returns an op if a command was chosen.
    fn menu_keys(
        &mut self,
        ui: &mut Ui,
        block: usize,
        query: &SlashQuery,
        matches: &[&Command],
    ) -> Option<Op> {
        let n = matches.len();
        self.menu_selected = self.menu_selected.min(n - 1);
        if consume(ui, Key::ArrowDown) {
            self.menu_selected = (self.menu_selected + 1) % n;
            self.scroll_menu = true;
        }
        if consume(ui, Key::ArrowUp) {
            self.menu_selected = (self.menu_selected + n - 1) % n;
            self.scroll_menu = true;
        }
        if consume(ui, Key::Escape) {
            self.menu_dismissed = Some((block, query.start));
        }
        if consume(ui, Key::Enter) || consume(ui, Key::Tab) {
            let action = matches[self.menu_selected].action.clone();
            return Some(Op::Run {
                block,
                query: query.clone(),
                action,
            });
        }
        None
    }

    /// Keys that cross block boundaries. Everything else goes to the text field.
    fn editing_keys(&mut self, ui: &mut Ui, i: usize, start: usize, end: usize) -> Option<Op> {
        if let Some(op) = copy_citation(self, ui, i, start, end) {
            return op;
        }
        if let Some(op) = self.formatting_keys(ui, i, start, end) {
            return Some(op);
        }
        if let Some(deeper) = take_tab(ui) {
            // In code, Tab types an indent inside the code instead.
            if deeper && matches!(self.doc.blocks[i].kind, BlockKind::Code { .. }) {
                let mut text = self.doc.blocks[i].text.clone();
                let (from, to) = (
                    ops::char_to_byte(&text, start),
                    ops::char_to_byte(&text, end),
                );
                text.replace_range(from..to, "    ");
                return Some(Op::SetText {
                    block: i,
                    text,
                    char: start + 4,
                    end: start + 4,
                });
            }
            // In a paragraph, Tab indents just the line(s) at the caret.
            if self.doc.blocks[i].kind == BlockKind::Paragraph {
                let mut text = self.doc.blocks[i].text.clone();
                let (char, end) = ops::indent_lines(&mut text, start, end, deeper);
                return Some(Op::SetText {
                    block: i,
                    text,
                    char,
                    end,
                });
            }
            return Some(Op::Indent {
                first: i,
                last: i,
                deeper,
                start,
                end,
            });
        }
        if !matches!(self.doc.blocks[i].kind, BlockKind::Code { .. }) {
            // A multi-line paste is blocks (a list, several paragraphs), not
            // characters dropped into this line.
            if let Some(text) = take_block_paste(ui) {
                return Some(Op::InsertAt {
                    block: i,
                    start,
                    end,
                    text,
                });
            }
            if let Some(url) = take_pasted_url(ui) {
                return Some(Op::InsertLink {
                    block: i,
                    start,
                    end,
                    url,
                });
            }
        }

        // Shift+Enter in a paragraph: a new line as indented as this one.
        if self.doc.blocks[i].kind == BlockKind::Paragraph
            && consume_mods(ui, Modifiers::SHIFT, Key::Enter)
        {
            let mut text = self.doc.blocks[i].text.clone();
            let char = ops::soft_break(&mut text, start, end);
            return Some(Op::SetText {
                block: i,
                text,
                char,
                end: char,
            });
        }

        let block = &self.doc.blocks[i];
        let len = block.text.chars().count();
        let is_code = matches!(block.kind, BlockKind::Code { .. });
        let shift = ui.input(|input| input.modifiers.shift);
        // Where the first line's text starts, past any hidden indent tabs.
        let text_start = if block.kind == BlockKind::Paragraph {
            ops::leading_tabs(&block.text)
        } else {
            0
        };
        let collapsed = start == end;
        let row = self.galleys.get(&i).map(|g| {
            let row = g.layout_from_cursor(CCursor::new(start)).row;
            (row, g.rows.len())
        });

        if !shift && !is_code && consume(ui, Key::Enter) {
            return Some(Op::Split {
                block: i,
                start,
                end,
            });
        }
        // In code, Enter on a trailing empty line leaves the block.
        if is_code
            && !shift
            && collapsed
            && start == len
            && block.text.ends_with('\n')
            && consume(ui, Key::Enter)
        {
            return Some(Op::ExitCode(i));
        }
        if collapsed && start == 0 && consume(ui, Key::Backspace) {
            return Some(Op::Backspace(i));
        }
        for (key, forward) in [(Key::Backspace, false), (Key::Delete, true)] {
            if collapsed && !is_code && ui.input(|input| input.key_pressed(key)) {
                let mut text = block.text.clone();
                let deleted = citations::delete_ref_at_edge(&mut text, start, forward)
                    .or_else(|| ops::delete_at_link_edge(&mut text, start, forward))
                    .or_else(|| marks::delete_at_edge(&mut text, start, forward));
                if let Some(char) = deleted {
                    consume(ui, key);
                    return Some(Op::SetText {
                        block: i,
                        text,
                        char,
                        end: char,
                    });
                }
            }
        }
        if collapsed && start == len && consume(ui, Key::Delete) {
            return Some(Op::DeleteForward(i));
        }

        let prev = self.neighbor_text(i, false);
        let next = self.neighbor_text(i, true);
        let end_of = |b: usize| Caret {
            block: b,
            char: self.doc.blocks[b].text.chars().count(),
        };
        let on_first_row = row.is_none_or(|(r, _)| r == 0);
        let on_last_row = row.is_none_or(|(r, rows)| r + 1 >= rows);

        if shift {
            return self.extend_out_of_block(ui, i);
        }
        if let Some(p) = prev {
            if (collapsed && start <= text_start && consume(ui, Key::ArrowLeft))
                || (on_first_row && consume(ui, Key::ArrowUp))
            {
                return Some(Op::Focus(end_of(p)));
            }
        }
        if let Some(n) = next {
            if (collapsed && start == len && consume(ui, Key::ArrowRight))
                || (on_last_row && consume(ui, Key::ArrowDown))
            {
                return Some(Op::Focus(Caret { block: n, char: 0 }));
            }
        }
        None
    }

    /// Shift+arrow at the edge of a block starts a selection that continues
    /// into the neighboring block. Inside the block, the text field handles it.
    fn extend_out_of_block(&mut self, ui: &mut Ui, i: usize) -> Option<Op> {
        let range = TextEdit::load_state(ui.ctx(), self.block_id(i))?
            .cursor
            .char_range()?;
        let (anchor, head) = (
            Caret {
                block: i,
                char: range.secondary.index.0,
            },
            Caret {
                block: i,
                char: range.primary.index.0,
            },
        );
        let len = self.doc.blocks[i].text.chars().count();
        let (row, rows) = self.galleys.get(&i).map_or((0, 1), |g| {
            (
                g.layout_from_cursor(CCursor::new(head.char)).row,
                g.rows.len(),
            )
        });
        let last = self.doc.blocks.len() - 1;
        // Only take the keys that leave this block; the rest (like
        // Cmd+Shift+Left to the line start) the text field handles itself.
        let crosses = |key: Key, m: Modifiers| match (Motion::of(key, m), key) {
            (Motion::Document, _) => last > 0,
            (Motion::Row, Key::ArrowUp) => i > 0 && row == 0,
            (Motion::Row, Key::ArrowDown) => i < last && row + 1 >= rows,
            (Motion::Char | Motion::Word, Key::ArrowLeft) => i > 0 && head.char == 0,
            (Motion::Char | Motion::Word, Key::ArrowRight) => i < last && head.char == len,
            _ => false,
        };
        let (key, m) = take_arrow(ui, |key, m| m.shift && crosses(key, m))?;
        let new_head = self.move_head(head, key, m);
        Some(Op::Select(Selection::new(anchor, new_head)))
    }

    /// Where an arrow key (with its modifiers) moves `head`, anywhere in the
    /// document.
    fn move_head(&self, head: Caret, key: Key, modifiers: Modifiers) -> Caret {
        let forward = matches!(key, Key::ArrowRight | Key::ArrowDown);
        match Motion::of(key, modifiers) {
            Motion::Char => selection::step(&self.doc, head, forward),
            Motion::Word => selection::word_step(&self.doc, head, forward),
            Motion::Line => self.line_edge(head, forward),
            Motion::Row => self.row_step(head, forward),
            Motion::Document if forward => Selection::all(&self.doc).head,
            Motion::Document => Caret { block: 0, char: 0 },
        }
    }

    /// The start or end of the visual line `caret` is on.
    fn line_edge(&self, caret: Caret, forward: bool) -> Caret {
        let Some(galley) = self.galleys.get(&caret.block) else {
            let len = self.doc.blocks[caret.block].text.chars().count();
            return Caret {
                char: if forward { len } else { 0 },
                ..caret
            };
        };
        let cursor = CCursor::new(caret.char);
        let edge = if forward {
            galley.cursor_end_of_row(&cursor)
        } else {
            galley.cursor_begin_of_row(&cursor)
        };
        Caret {
            char: edge.index.0,
            ..caret
        }
    }

    /// Cmd+A and mouse presses/drags, which can start or change a selection.
    fn selection_input(&mut self, ui: &mut Ui) -> Option<Op> {
        let focused = (0..self.doc.blocks.len()).find(|&i| {
            let id = self.block_id(i);
            ui.memory(|m| m.has_focus(id))
        });
        if (focused.is_some() || self.selection.is_some())
            && consume_mods(ui, Modifiers::COMMAND, Key::A)
        {
            return Some(Op::Select(Selection::all(&self.doc)));
        }

        let (pressed, down, released, pos, shift) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
            )
        });
        // Only presses on the blocks (not the sidebar, a popup menu, or the
        // note's properties above the first block).
        let top = self
            .layouts
            .get(&0)
            .map_or(f32::INFINITY, |l| l.0.min.y - 4.0);
        let on_page = pos.is_some_and(|p| {
            p.y >= top
                && ui.clip_rect().contains(p)
                && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
        });
        let at = pos.and_then(|p| self.caret_at(p));

        if pressed {
            self.drag_anchor = None;
            if !on_page {
                return None;
            }
            let at = at?;
            // Shift+click extends from the caret or the current selection.
            let from = self.selection.map(|s| s.anchor).or_else(|| {
                let i = focused?;
                let range = TextEdit::load_state(ui.ctx(), self.block_id(i))?
                    .cursor
                    .char_range()?;
                Some(Caret {
                    block: i,
                    char: range.secondary.index.0,
                })
            });
            if let (true, Some(anchor)) = (shift, from) {
                if anchor.block != at.block || self.selection.is_some() {
                    self.drag_anchor = Some(anchor);
                    return Some(Op::Select(Selection::new(anchor, at)));
                }
            }
            self.drag_anchor = Some(at);
            // The text fields were inactive during the selection, so egui won't
            // route this click to one; put the caret where it landed ourselves.
            if self.selection.take().is_some() {
                return Some(Op::Focus(at));
            }
            return None;
        }

        let anchor = self.drag_anchor?;
        if down {
            let at = at?;
            // Crossing into another block turns the drag into a document selection.
            if at.block != anchor.block || self.selection.is_some() {
                return Some(Op::Select(Selection::new(anchor, at)));
            }
        } else {
            self.drag_anchor = None;
            // A drag that ended back inside one block hands off to that block.
            if released {
                if let Some(sel) = self
                    .selection
                    .filter(|s| !s.spans_blocks() && !s.is_empty())
                {
                    self.selection = None;
                    self.pending_selection_end = Some(sel.head.char);
                    self.pending_caret = Some(sel.anchor);
                }
            }
        }
        None
    }

    /// Draws the find bar (when open) with its top-right corner at
    /// `top_right`, and selects the current match when it closes.
    pub fn show_find_bar(&mut self, ctx: &egui::Context, top_right: Pos2, palette: &Palette) {
        if let Some(m) = self.find.show(ctx, top_right, palette, find_bar::NOTE_HINT) {
            self.selection = None;
            self.pending_caret = Some(Caret {
                block: m.block,
                char: m.start,
            });
            self.pending_selection_end = Some(m.end);
        }
    }

    /// A caret as tall as the text on its line (not the whole line slot,
    /// which includes the spacing below), blinking like egui's.
    fn paint_caret(
        &mut self,
        ui: &Ui,
        block: usize,
        kind: &BlockKind,
        output: &egui::text_edit::TextEditOutput,
        cursor: CCursor,
        stroke: Stroke,
    ) {
        if !ui.input(|i| i.focused) {
            return; // like egui: no caret while the window is in the background
        }
        let now = ui.input(|i| i.time);
        let key = (block, cursor.index.0);
        let moved_at = match self.caret_moved {
            Some((k, t)) if k == key => t,
            _ => {
                self.caret_moved = Some((key, now));
                now
            }
        };
        let style = &ui.visuals().text_cursor;
        if style.blink {
            let (on, off) = (f64::from(style.on_duration), f64::from(style.off_duration));
            let phase = (now - moved_at) % (on + off);
            let wait = if phase < on {
                on - phase
            } else {
                on + off - phase
            };
            ui.ctx().request_repaint_after_secs(wait as f32);
            if phase >= on {
                return;
            }
        }
        let rect = caret_rect(&output.galley, cursor, theme::text_height(ui, kind))
            .translate(output.galley_pos.to_vec2());
        ui.painter()
            .line_segment([rect.center_top(), rect.center_bottom()], stroke);
    }

    /// Keys while a document selection is active.
    fn selection_keys(&mut self, ui: &mut Ui) -> Option<Op> {
        let sel = self.selection?;
        if let Some((kind, color)) = take_mark_shortcut(ui) {
            return Some(Op::Mark {
                selection: true,
                block: 0,
                start: 0,
                end: 0,
                kind,
                color: Some(color),
                toggle: true,
            });
        }
        let (start, end) = sel.range();
        let extend = |head| Some(Op::Select(Selection::new(sel.anchor, head)));
        if let Some((key, m)) = take_arrow(ui, |_, _| true) {
            if m.shift {
                return extend(self.move_head(sel.head, key, m));
            }
            // Without Shift, the selection collapses toward the arrow's side.
            let back = matches!(key, Key::ArrowLeft | Key::ArrowUp);
            return Some(Op::Focus(if back { start } else { end }));
        }
        if consume(ui, Key::Escape) {
            return Some(Op::Focus(sel.head));
        }
        if consume(ui, Key::Backspace) || consume(ui, Key::Delete) {
            return Some(Op::DeleteSelection);
        }
        if let Some(deeper) = take_tab(ui) {
            let (start, end) = sel.range();
            return Some(Op::Indent {
                first: start.block,
                last: end.block,
                deeper,
                start: 0,
                end: 0,
            });
        }
        if consume(ui, Key::Enter) {
            return Some(Op::SplitSelection);
        }

        // Typing, pasting, and the clipboard.
        let taken = ui.input_mut(|i| {
            let mut taken = Vec::new();
            i.events.retain(|e| match e {
                egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Copy
                | egui::Event::Cut => {
                    taken.push(e.clone());
                    false
                }
                _ => true,
            });
            taken
        });
        let mut op = None;
        for event in taken {
            match event {
                egui::Event::Copy => ui.ctx().copy_text(selection::to_markdown(&self.doc, &sel)),
                egui::Event::Cut => {
                    ui.ctx().copy_text(selection::to_markdown(&self.doc, &sel));
                    op = Some(Op::DeleteSelection);
                }
                egui::Event::Text(text) | egui::Event::Paste(text) => {
                    op = Some(Op::ReplaceSelection(text));
                }
                _ => {}
            }
        }
        op
    }

    /// The document position under a screen point, from last frame's layout.
    fn caret_at(&self, pos: Pos2) -> Option<Caret> {
        let last = self.doc.blocks.len() - 1;
        for i in 0..=last {
            let (rect, origin) = *self.layouts.get(&i)?;
            if pos.y <= rect.max.y {
                let char = self
                    .galleys
                    .get(&i)
                    .map_or(0, |g| g.cursor_from_pos(pos - origin).index.0);
                return Some(Caret { block: i, char });
            }
        }
        Some(Selection::all(&self.doc).head)
    }

    /// One visual row up or down from `caret`, keeping the horizontal
    /// position, and crossing into neighboring blocks.
    fn row_step(&self, caret: Caret, down: bool) -> Caret {
        let Some(galley) = self.galleys.get(&caret.block) else {
            return selection::step(&self.doc, caret, down);
        };
        let origin = self.layouts.get(&caret.block).map_or(Pos2::ZERO, |l| l.1);
        let cursor = CCursor::new(caret.char);
        let x = galley.pos_from_cursor(cursor).center().x + origin.x;
        let row = galley.layout_from_cursor(cursor).row;
        let in_block = if down {
            (row + 1 < galley.rows.len())
                .then(|| galley.cursor_down_one_row(&cursor, Some(x - origin.x)))
        } else {
            (row > 0).then(|| galley.cursor_up_one_row(&cursor, Some(x - origin.x)))
        };
        if let Some((moved, _)) = in_block {
            return Caret {
                char: moved.index.0,
                ..caret
            };
        }

        let Some(target) = self.neighbor_block(caret.block, down) else {
            return if down {
                Selection::all(&self.doc).head
            } else {
                Caret { block: 0, char: 0 }
            };
        };
        let (Some(galley), Some(&(_, origin))) =
            (self.galleys.get(&target), self.layouts.get(&target))
        else {
            return Caret {
                block: target,
                char: 0,
            };
        };
        let row = if down {
            galley.rows.first()
        } else {
            galley.rows.last()
        };
        let y = row.map_or(0.0, |r| (r.min_y() + r.max_y()) / 2.0);
        Caret {
            block: target,
            char: galley.cursor_from_pos(vec2(x - origin.x, y)).index.0,
        }
    }

    /// Bold, italic, and the other shortcuts people expect from a text editor.
    /// Shift-modified shortcuts are checked first because egui treats an extra
    /// Shift as a match for the unmodified shortcut.
    fn formatting_keys(&mut self, ui: &mut Ui, i: usize, start: usize, end: usize) -> Option<Op> {
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let cmd_alt = Modifiers::COMMAND | Modifiers::ALT;
        let kind = |kind| Op::Kind {
            block: i,
            kind,
            start,
            end,
        };
        let style = |marker| Op::Style {
            block: i,
            start,
            end,
            marker,
        };

        if consume_mods(ui, cmd_shift, Key::Num7) {
            return Some(kind(BlockKind::Numbered));
        }
        if consume_mods(ui, cmd_shift, Key::Num8) {
            return Some(kind(BlockKind::Bullet));
        }
        if consume_mods(ui, cmd_shift, Key::Num9) {
            return Some(kind(BlockKind::Todo { checked: false }));
        }
        if consume_mods(ui, cmd_shift, Key::Period) {
            return Some(kind(BlockKind::Quote));
        }
        if consume_mods(ui, cmd_alt, Key::Num0) {
            return Some(kind(BlockKind::Paragraph));
        }
        if consume_mods(ui, cmd_alt, Key::Num1) {
            return Some(kind(BlockKind::Heading(1)));
        }
        if consume_mods(ui, cmd_alt, Key::Num2) {
            return Some(kind(BlockKind::Heading(2)));
        }
        if consume_mods(ui, cmd_alt, Key::Num3) {
            return Some(kind(BlockKind::Heading(3)));
        }
        if consume_mods(ui, cmd_alt, Key::Num8) {
            return Some(kind(BlockKind::Code {
                lang: String::new(),
            }));
        }

        let is_code = matches!(self.doc.blocks[i].kind, BlockKind::Code { .. });
        if !is_code {
            if let Some((kind, color)) = take_mark_shortcut(ui) {
                return Some(Op::Mark {
                    selection: false,
                    block: i,
                    start,
                    end,
                    kind,
                    color: Some(color),
                    toggle: true,
                });
            }
            if consume_mods(ui, cmd_shift, Key::X) {
                return Some(style("~~"));
            }
            if consume_mods(ui, cmd, Key::B) {
                return Some(style("**"));
            }
            if consume_mods(ui, cmd, Key::I) {
                return Some(style("*"));
            }
            if consume_mods(ui, cmd, Key::E) {
                return Some(style("`"));
            }
        }
        if matches!(self.doc.blocks[i].kind, BlockKind::Todo { .. })
            && consume_mods(ui, cmd, Key::Enter)
        {
            return Some(Op::ToggleTodo(i));
        }
        None
    }

    /// Bullets, numbers, and checkboxes to the left of a block.
    fn gutter(&self, ui: &mut Ui, i: usize, kind: &BlockKind, palette: &Palette) -> Option<Op> {
        let line = theme::line_height(kind);
        let size = vec2(theme::GUTTER, line);
        match kind {
            BlockKind::Bullet => {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let center = rect.left_center() + vec2(9.0, 0.0);
                ui.painter().circle_filled(center, 2.8, palette.text);
            }
            BlockKind::Numbered => {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().text(
                    rect.left_center() + vec2(2.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{}.", self.doc.list_number(i)),
                    FontId::proportional(16.0),
                    palette.text,
                );
            }
            BlockKind::Todo { checked } => {
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                let square = egui::Rect::from_center_size(
                    rect.left_center() + vec2(9.0, 0.0),
                    vec2(16.0, 16.0),
                );
                let painter = ui.painter();
                if *checked {
                    painter.rect_filled(square, 3.0, palette.accent);
                    let points = [
                        square.left_center() + vec2(3.5, 0.5),
                        square.center_bottom() + vec2(-1.5, -4.0),
                        square.right_top() + vec2(-3.5, 4.0),
                    ];
                    painter.line(points.to_vec(), Stroke::new(1.8, Color32::WHITE));
                } else {
                    painter.rect_stroke(
                        square,
                        3.0,
                        Stroke::new(1.5, palette.text),
                        egui::StrokeKind::Inside,
                    );
                }
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    return Some(Op::ToggleTodo(i));
                }
            }
            BlockKind::Quote => {
                ui.allocate_exact_size(vec2(QUOTE_INDENT, line), Sense::hover());
            }
            _ => {}
        }
        None
    }

    /// The note's citations, below a divider at the end of the page.
    /// The list starts collapsed. Clicking the header opens or closes it;
    /// clicking a number in the text opens it and jumps there.
    /// Clicking a number in the list goes back to where it's cited; links open.
    fn citation_list(&mut self, ui: &mut Ui, palette: &Palette) {
        ui.add_space(40.0);
        let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().hline(
            rule.x_range(),
            rule.center().y,
            Stroke::new(1.0, palette.border),
        );
        ui.add_space(10.0);

        let width = ui.available_width();
        let (header, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Citations"));
        if response.hovered() {
            ui.painter().rect_filled(header, 6.0, palette.hover);
        }
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            self.citations_open = !self.citations_open;
        }
        // A superscript click asks to show that citation, so open the list.
        if self.jump_to_citation.is_some() {
            self.citations_open = true;
        }
        let y = header.center().y;
        crate::icons::chevron(
            ui.painter(),
            header.left_center() + vec2(10.0, 0.0),
            self.citations_open,
            palette.faint,
        );
        let title = theme::semibold(ui, "Citations", 14.0, palette.text);
        let count_x = header.left() + 22.0 + title.size().x + 8.0;
        ui.painter().galley(
            egui::pos2(header.left() + 22.0, y - title.size().y / 2.0),
            title,
            palette.text,
        );
        let count = self.doc.citations.len().to_string();
        ui.painter().text(
            egui::pos2(count_x, y),
            egui::Align2::LEFT_CENTER,
            count,
            FontId::proportional(13.0),
            palette.faint,
        );
        if !self.citations_open {
            return;
        }
        ui.add_space(6.0);

        const NUMBER_WIDTH: f32 = 30.0;
        let citations = self.doc.citations.clone();
        // Keep every citation value on the same vertical line. The previous
        // per-row measurement made long scripture references push only their
        // own value to the right.
        let label_width = citations
            .iter()
            .map(|citation| {
                if citations::is_numbered(&citation.id) {
                    NUMBER_WIDTH
                } else {
                    ui.painter()
                        .layout_no_wrap(
                            citation.id.clone(),
                            FontId::proportional(13.0),
                            palette.faint,
                        )
                        .size()
                        .x
                        + 14.0
                }
            })
            .max_by(f32::total_cmp)
            .unwrap_or(NUMBER_WIDTH)
            .clamp(NUMBER_WIDTH, (width * 0.46).max(NUMBER_WIDTH));
        for citation in &citations {
            let numbered = citations::is_numbered(&citation.id);
            let galley = theme::layout(
                ui,
                &citation.text,
                &BlockKind::Paragraph,
                false,
                false,
                (width - label_width).max(80.0),
            );
            let (rect, response) = ui
                .allocate_exact_size(vec2(width, galley.size().y.max(22.0) + 8.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Citation {}", citation.id),
                )
            });
            if self.jump_to_citation.as_deref() == Some(&citation.id) {
                self.jump_to_citation = None;
                self.highlighted_citation = Some(citation.id.clone());
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
            if self.highlighted_citation.as_deref() == Some(&citation.id) {
                let area = rect.expand2(vec2(8.0, 2.0));
                ui.painter()
                    .rect_filled(area, 6.0, palette.accent.gamma_multiply(0.18));
                ui.painter().rect_filled(
                    Rect::from_min_size(area.min, vec2(3.0, area.height())),
                    CornerRadius {
                        nw: 6,
                        sw: 6,
                        ne: 0,
                        se: 0,
                    },
                    palette.accent,
                );
            }

            let text_pos = rect.min + vec2(label_width, 4.0);
            let text_rect = Rect::from_min_size(text_pos, galley.size());
            let number_rect =
                Rect::from_min_size(rect.min, vec2((label_width - 6.0).max(1.0), 26.0));
            if numbered {
                ui.painter().text(
                    number_rect.right_top() + vec2(0.0, 4.0),
                    egui::Align2::RIGHT_TOP,
                    format!("{}.", citation.id),
                    FontId::proportional(15.0),
                    palette.faint,
                );
            } else {
                ui.painter().with_clip_rect(number_rect).text(
                    number_rect.left_top() + vec2(0.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    &citation.id,
                    FontId::proportional(13.0),
                    palette.faint,
                );
            }
            // A selectable Label is needed here instead of painting the
            // galley directly: egui's label selection machinery can then
            // handle drag-to-highlight and copy for citation text.
            let text_response =
                ui.put(text_rect, egui::Label::new(galley.clone()).selectable(true));
            if (response.secondary_clicked() || text_response.secondary_clicked())
                && scriptures::find_verses(&citation.id)
                    .iter()
                    .any(|hit| hit.label == citation.id)
            {
                let position = ui
                    .input(|input| input.pointer.interact_pos())
                    .unwrap_or(rect.right_bottom());
                self.scripture_citation_menu =
                    Some(ScriptureCitationMenu::new(citation.id.clone(), position));
            }
            let on_number = ui.rect_contains_pointer(number_rect);
            let url = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| rect.contains(*p))
                .and_then(|p| link_under(&galley, p - text_pos, &citation.text));
            if on_number || url.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() || text_response.clicked() {
                if let Some(url) = url {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                } else if on_number {
                    if let Some(at) = self.ref_rects.get(&citation.id) {
                        ui.scroll_to_rect(*at, Some(egui::Align::Center));
                    }
                } else {
                    self.highlighted_citation = Some(citation.id.clone());
                }
            }
            if on_number {
                response.on_hover_text("Back to where it's cited");
            }
        }
    }

    /// Draws an image block. Clicking selects it; a button on hover deletes it.
    fn image_block(
        &mut self,
        ui: &mut Ui,
        i: usize,
        path: &Path,
        alt: &str,
        indent: f32,
        palette: &Palette,
    ) -> Option<Op> {
        let texture = image_texture(ui.ctx(), path);
        let width = ui.available_width() - indent;
        let size = match &texture {
            Some(texture) => {
                // Screenshots are taken at the display's pixel density.
                let natural = texture.size_vec2() / ui.ctx().pixels_per_point();
                let scale = (width / natural.x)
                    .min(IMAGE_MAX_HEIGHT / natural.y)
                    .min(1.0);
                natural * scale
            }
            None => vec2(width, 56.0),
        };
        ui.add_space(6.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::hover());
        let rect = Rect::from_min_size(row.min + vec2(indent, 0.0), size);
        let response = ui.interact(rect, Id::new(("image", &self.note_id, i)), Sense::click());
        ui.add_space(8.0);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Image,
                true,
                if alt.is_empty() { "Image" } else { alt },
            )
        });
        self.layouts.insert(i, (rect, rect.left_top()));
        self.galleys.remove(&i);

        match &texture {
            Some(texture) => {
                egui::Image::from_texture(texture)
                    .corner_radius(CornerRadius::same(6))
                    .paint_at(ui, rect);
            }
            None => {
                ui.painter().rect_filled(rect, 6.0, palette.code_bg);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("Image not found: {}", path.display()),
                    FontId::proportional(13.0),
                    palette.faint,
                );
            }
        }

        let in_selection = self
            .selection
            .and_then(|s| s.in_block(&self.doc, i))
            .is_some();
        let selected = self.selected_image == Some(i) || in_selection;
        if selected {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                8.0,
                Stroke::new(2.0, palette.accent),
                egui::StrokeKind::Outside,
            );
        }

        // Hovered rather than `response.hovered()`: the button sits on top.
        if ui.rect_contains_pointer(rect) || self.selected_image == Some(i) {
            let button = Rect::from_min_size(rect.right_top() + vec2(-36.0, 8.0), vec2(28.0, 28.0));
            let delete = ui.interact(button, Id::new(("delete-image", i)), Sense::click());
            delete.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Delete image")
            });
            let fill = Color32::from_black_alpha(if delete.hovered() { 200 } else { 140 });
            ui.painter().rect_filled(button, 6.0, fill);
            crate::icons::trash(ui.painter(), button.center(), Color32::WHITE);
            if delete.on_hover_text("Delete image").clicked() {
                return Some(Op::RemoveBlock(i));
            }
        }

        if response.clicked() {
            return Some(Op::SelectImage(i));
        }
        if self.selected_image == Some(i)
            && ui.input(|input| input.pointer.any_pressed())
            && !ui.rect_contains_pointer(rect)
        {
            self.selected_image = None;
        }
        None
    }

    /// Keys while an image is selected.
    fn image_keys(&mut self, ui: &mut Ui) -> Option<Op> {
        let i = self.selected_image?;
        if i >= self.doc.blocks.len() || self.doc.blocks[i].kind.has_text() {
            self.selected_image = None;
            return None;
        }
        if consume(ui, Key::Backspace) || consume(ui, Key::Delete) {
            return Some(Op::RemoveBlock(i));
        }
        if consume(ui, Key::Escape) {
            self.selected_image = None;
            return None;
        }
        if let Some(deeper) = take_tab(ui) {
            return Some(Op::Indent {
                first: i,
                last: i,
                deeper,
                start: 0,
                end: 0,
            });
        }
        for (key, forward) in [
            (Key::Enter, true),
            (Key::ArrowDown, true),
            (Key::ArrowRight, true),
            (Key::ArrowUp, false),
            (Key::ArrowLeft, false),
        ] {
            if consume(ui, key) {
                return Some(Op::FocusBeside { block: i, forward });
            }
        }
        None
    }

    /// Keeps the caret in block `i` out of hidden link markup.
    fn skip_link_markup(&mut self, ui: &Ui, i: usize, id: Id) {
        let Some(mut state) = TextEdit::load_state(ui.ctx(), id) else {
            return;
        };
        let Some(range) = state.cursor.char_range() else {
            return;
        };
        let caret = range.primary.index.0;
        let prev = match self.last_caret {
            Some((block, char)) if block == i => char,
            // Arriving from elsewhere counts as a jump.
            _ => usize::MAX,
        };
        let block = &self.doc.blocks[i];
        let indent_tabs = block.kind == BlockKind::Paragraph;
        let snapped = ops::skip_hidden(&block.text, prev, caret, indent_tabs);
        if snapped != caret {
            let secondary = if range.is_empty() {
                CCursor::new(snapped)
            } else {
                CCursor::new(range.secondary.index.0)
            };
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(secondary, CCursor::new(snapped))));
            TextEdit::store_state(ui.ctx(), id, state);
        }
        self.last_caret = Some((i, snapped));
    }

    /// Runs a link-menu action. A replacement comes back as an operation.
    fn apply_link_choice(
        &mut self,
        choice: link_menu::Choice,
        target: LinkTarget,
        ui: &Ui,
    ) -> Option<Op> {
        let text = self
            .doc
            .blocks
            .get(target.block)
            .map(|block| block.text.clone())
            .filter(|text| target.still_in(text));
        match choice {
            link_menu::Choice::Copy => {
                ui.ctx().copy_text(target.url.clone());
                None
            }
            link_menu::Choice::Edit => {
                if let Some(text) = text {
                    self.link_form = Some(LinkForm::new(target, &text));
                }
                None
            }
            link_menu::Choice::Remove => text.map(|text| {
                let label = text[target.label.clone()].to_string();
                let (text, char) = target.replace(&text, &label);
                Op::SetText {
                    block: target.block,
                    text,
                    char,
                    end: char,
                }
            }),
            link_menu::Choice::ConvertToCitation => {
                if let Some(text) = text {
                    let title = if target.is_bare() {
                        String::new()
                    } else {
                        inline::plain_text(&text[target.label.clone()])
                    };
                    self.citation_form = Some(CitationForm::from_link(target, title));
                }
                None
            }
            link_menu::Choice::Mark(choice) => text.map(|text| {
                let start = ops::byte_to_char(&text, target.label.start);
                let end = ops::byte_to_char(&text, target.label.end);
                Op::Mark {
                    selection: false,
                    block: target.block,
                    start,
                    end,
                    kind: choice.kind,
                    color: choice.color,
                    toggle: choice.toggle,
                }
            }),
        }
    }

    /// Opens a link on click, straight away, whether or not the block is
    /// being edited.
    fn link_click(
        &mut self,
        ui: &Ui,
        i: usize,
        output: &egui::text_edit::TextEditOutput,
        focused: bool,
        notes: &[NoteMeta],
    ) -> Option<LinkPress> {
        let pos = ui.input(|input| input.pointer.hover_pos());
        let text = &self.doc.blocks[i].text;
        let at = pos
            .filter(|&p| output.response.rect.contains(p))
            .and_then(|p| char_under(&output.galley, p - output.galley_pos, text));
        let target = at.and_then(|at| {
            let citation = citations::ref_at(text, at)
                .filter(|id| self.doc.citations.iter().any(|c| &c.id == id));
            match citation {
                Some(id) => Some(Target::Citation(id)),
                None => inline::link_at(text, at).map(|url| match links::note_id(&url) {
                    Some(id) => Target::Note(id),
                    None => Target::Url(url),
                }),
            }
        });
        let pressed = ui.input(|input| input.pointer.primary_pressed());
        if let Some(target) = &target {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            let hint = match target {
                Target::Url(url) => url.clone(),
                Target::Note(id) => notes
                    .iter()
                    .find(|note| &note.id == id)
                    .map(|note| {
                        let folder = note.folder();
                        if folder.is_empty() {
                            note.title.clone()
                        } else {
                            format!("{folder} — {}", note.title)
                        }
                    })
                    .unwrap_or_else(|| "This note is no longer in the library".to_string()),
                Target::Citation(id) => self
                    .doc
                    .citations
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| inline::plain_text(&c.text))
                    .unwrap_or_default(),
            };
            output
                .response
                .response
                .clone()
                .on_hover_text_at_pointer(hint);
        }
        // Decide on press: by the release, the click has focused the block.
        // (Cleared on every press in `show`; only the block with the link sets it.)
        if let (true, Some(target)) = (pressed, target) {
            self.link_press = Some(LinkPress {
                block: i,
                target,
                editing: focused,
            });
        }
        let clicked = output.response.clicked();
        self.link_press.take_if(|press| press.block == i && clicked)
    }

    fn show_menu(&mut self, ui: &Ui, menu: &OpenMenu, palette: &Palette) -> Option<Action> {
        let mut chosen = None;
        egui::Area::new(Id::new(("slash-menu", &self.note_id)))
            .order(Order::Foreground)
            .fixed_pos(menu.anchor)
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(palette.menu_bg)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(Shadow {
                        offset: [0, 6],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_black_alpha(40),
                    })
                    .inner_margin(Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(MENU_WIDTH);
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for (n, command) in menu.matches.iter().enumerate() {
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), MENU_ROW_HEIGHT),
                                        Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            &command.label,
                                        )
                                    });
                                    if response.hovered()
                                        && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                                    {
                                        self.menu_selected = n;
                                    }
                                    // The click also unfocuses the text field; the caret
                                    // set by the command puts focus back next frame.
                                    if response.clicked() {
                                        chosen = Some(command.action.clone());
                                    }
                                    if n == self.menu_selected {
                                        ui.painter().rect_filled(rect, 5.0, palette.menu_selected);
                                        if std::mem::take(&mut self.scroll_menu) {
                                            response.scroll_to_me(None);
                                        }
                                    }
                                    ui.painter().text(
                                        rect.left_center() + vec2(10.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        &command.label,
                                        FontId::proportional(14.0),
                                        palette.text,
                                    );
                                    ui.painter().text(
                                        rect.right_center() - vec2(10.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        &command.hint,
                                        FontId::proportional(12.0),
                                        palette.faint,
                                    );
                                }
                            });
                    });
            });
        chosen
    }

    fn apply(&mut self, op: Op) -> Option<Event> {
        let outline_before = self.outline_keys();
        let doc = &mut self.doc;
        let mut event = Some(Event::Changed);
        self.pending_selection_end = None;
        if !matches!(op, Op::SelectImage(_)) {
            self.selected_image = None;
        }
        let caret = match op {
            Op::InsertLink {
                block,
                start,
                end,
                url,
            } => {
                let text = &mut doc.blocks[block].text;
                let (from, to) = (ops::char_to_byte(text, start), ops::char_to_byte(text, end));
                let selected = text[from..to].to_string();
                let link = links::markdown(&url, (from < to).then_some(selected.as_str()));
                text.replace_range(from..to, &link);
                Some(Caret {
                    block,
                    char: start + link.chars().count(),
                })
            }
            Op::Indent {
                first,
                last,
                deeper,
                start,
                end,
            } => {
                ops::indent_blocks(doc, first, last, deeper);
                if self.selection.is_some() || !doc.blocks[first].kind.has_text() {
                    // Keep the document selection (or picture) selected.
                    if !doc.blocks[first].kind.has_text() && self.selection.is_none() {
                        self.selected_image = Some(first);
                    }
                    None
                } else {
                    self.pending_selection_end = Some(end);
                    Some(Caret {
                        block: first,
                        char: start,
                    })
                }
            }
            Op::ConvertLinkToCitation { target, text } => {
                let current = &doc.blocks.get(target.block)?.text;
                if !target.still_in(current) {
                    return None;
                }
                // A bare address is replaced by the number; a named link
                // keeps its words, now plain, with the number after them.
                let label = if target.is_bare() {
                    String::new()
                } else {
                    current[target.label.clone()].to_string()
                };
                let (mut new_text, mut char) = target.replace(current, &label);
                if target.is_bare() {
                    let at = ops::char_to_byte(&new_text, char);
                    if new_text[..at].ends_with(' ') {
                        new_text.remove(at - 1);
                        char -= 1;
                    }
                }
                doc.blocks[target.block].text = new_text;
                let caret = citations::insert(doc, target.block, char, &text);
                self.caret_after_form = Some((caret, 1));
                None
            }
            Op::InsertCitation { block, char, text } => {
                let block = block.min(doc.blocks.len() - 1);
                let char = char.min(doc.blocks[block].text.chars().count());
                let caret = citations::insert(doc, block, char, &text);
                self.caret_after_form = Some((caret, 1));
                None
            }
            Op::InsertScripture {
                block,
                at,
                label,
                text,
            } => {
                let block = block.min(doc.blocks.len() - 1);
                let at = at.min(doc.blocks[block].text.chars().count());
                Some(citations::insert_labeled(doc, block, at, &label, &text))
            }
            Op::EditScripture {
                old_label,
                new_label,
                text,
            } => {
                let citation = doc.citations.iter_mut().find(|c| c.id == old_label)?;
                citation.id = new_label.clone();
                citation.text = text;
                for block in &mut doc.blocks {
                    let refs = citations::refs(&block.text)
                        .into_iter()
                        .filter(|reference| reference.id == old_label)
                        .map(|reference| reference.range)
                        .collect::<Vec<_>>();
                    for range in refs.into_iter().rev() {
                        block.text.replace_range(range, &format!("[^{new_label}]"));
                    }
                }
                None
            }
            Op::InsertNoteLink {
                block,
                at,
                id,
                title,
            } => {
                let block = block.min(doc.blocks.len() - 1);
                let text = &mut doc.blocks[block].text;
                let at = at.min(text.chars().count());
                let byte = ops::char_to_byte(text, at);
                let link = links::note_markdown(&id, &title);
                text.insert_str(byte, &link);
                Some(Caret {
                    block,
                    char: at + link.chars().count(),
                })
            }
            Op::SetText {
                block,
                text,
                char,
                end,
            } => {
                doc.blocks[block].text = text;
                if end != char {
                    self.pending_selection_end = Some(end);
                }
                Some(Caret { block, char })
            }
            Op::SelectImage(block) => {
                event = None;
                self.selection = None;
                self.selected_image = Some(block);
                None
            }
            Op::RemoveBlock(block) => {
                doc.blocks.remove(block);
                doc.ensure_not_empty();
                let prev = block
                    .checked_sub(1)
                    .filter(|&b| doc.blocks[b].kind.has_text());
                Some(match prev {
                    Some(b) => Caret {
                        block: b,
                        char: doc.blocks[b].text.chars().count(),
                    },
                    None => Caret {
                        block: block.min(doc.blocks.len() - 1),
                        char: 0,
                    },
                })
            }
            Op::FocusBeside { block, forward } => {
                event = None;
                let found = if forward {
                    doc.blocks[block + 1..]
                        .iter()
                        .position(|b| b.kind.has_text())
                        .map(|n| block + 1 + n)
                } else {
                    doc.blocks[..block].iter().rposition(|b| b.kind.has_text())
                };
                match (found, forward) {
                    (Some(b), true) => Some(Caret { block: b, char: 0 }),
                    (Some(b), false) => Some(Caret {
                        block: b,
                        char: doc.blocks[b].text.chars().count(),
                    }),
                    (None, true) => {
                        event = Some(Event::Changed);
                        doc.blocks.insert(block + 1, Block::paragraph(""));
                        Some(Caret {
                            block: block + 1,
                            char: 0,
                        })
                    }
                    (None, false) => {
                        self.selected_image = Some(block);
                        None
                    }
                }
            }
            Op::Split { block, start, end } => Some(ops::split_block(doc, block, start, end)),
            Op::Backspace(block) => ops::backspace_at_start(doc, block),
            Op::DeleteForward(block) => ops::delete_at_end(doc, block),
            Op::Focus(caret) => {
                event = None;
                self.selection = None;
                Some(caret)
            }
            Op::Select(sel) => {
                event = None;
                self.selection = Some(sel);
                None
            }
            Op::MoveSelection { destination } => {
                let sel = self.selection.take()?;
                let markdown = selection::to_markdown(doc, &sel);
                event = Some(Event::MoveSelection {
                    destination,
                    markdown,
                });
                Some(selection::delete(doc, &sel))
            }
            Op::DeleteSelection => {
                let sel = self.selection.take()?;
                Some(selection::delete(doc, &sel))
            }
            Op::ReplaceSelection(text) => {
                let sel = self.selection.take()?;
                Some(selection::replace(doc, &sel, &text))
            }
            Op::InsertAt {
                block,
                start,
                end,
                text,
            } => {
                let (start, end) = (start.min(end), start.max(end));
                let at = Caret { block, char: start };
                if start == end {
                    Some(selection::insert(doc, at, &text))
                } else {
                    Some(selection::replace(
                        doc,
                        &Selection::new(at, Caret { block, char: end }),
                        &text,
                    ))
                }
            }
            Op::SplitSelection => {
                let sel = self.selection.take()?;
                let at = selection::delete(doc, &sel);
                Some(ops::split_block(doc, at.block, at.char, at.char))
            }
            Op::ToggleTodo(block) => {
                if let BlockKind::Todo { checked } = &mut doc.blocks[block].kind {
                    *checked = !*checked;
                }
                None
            }
            Op::ExitCode(block) => {
                doc.blocks[block].text.pop();
                doc.blocks.insert(block + 1, Block::paragraph(""));
                Some(Caret {
                    block: block + 1,
                    char: 0,
                })
            }
            Op::Style {
                block,
                start,
                end,
                marker,
            } => {
                let (start, end) =
                    ops::toggle_inline(&mut doc.blocks[block].text, start, end, marker);
                self.pending_selection_end = Some(end);
                Some(Caret { block, char: start })
            }
            Op::Mark {
                selection,
                block,
                start,
                end,
                kind,
                color,
                toggle,
            } => {
                if selection {
                    if let Some(sel) = self.selection {
                        let before: Vec<String> =
                            doc.blocks.iter().map(|b| b.text.clone()).collect();
                        let sel = marks::edit_selection(doc, &sel, kind, color, toggle);
                        if doc
                            .blocks
                            .iter()
                            .zip(&before)
                            .all(|(b, old)| b.text == *old)
                        {
                            event = None;
                        }
                        self.selection = Some(sel);
                    } else {
                        event = None;
                    }
                    None
                } else if doc
                    .blocks
                    .get(block)
                    .is_some_and(|b| b.kind.has_text() && !matches!(b.kind, BlockKind::Code { .. }))
                {
                    let before = doc.blocks[block].text.clone();
                    let (start, end) =
                        marks::edit(&mut doc.blocks[block].text, start, end, kind, color, toggle);
                    if doc.blocks[block].text == before {
                        event = None;
                    }
                    self.pending_selection_end = Some(end);
                    Some(Caret { block, char: start })
                } else {
                    event = None;
                    None
                }
            }
            Op::ClearMarks {
                selection,
                block,
                start,
                end,
            } => {
                if selection {
                    if let Some(sel) = self.selection {
                        let before: Vec<String> =
                            doc.blocks.iter().map(|b| b.text.clone()).collect();
                        let sel =
                            marks::edit_selection(doc, &sel, MarkKind::Highlight, None, false);
                        let sel =
                            marks::edit_selection(doc, &sel, MarkKind::Underline, None, false);
                        if doc
                            .blocks
                            .iter()
                            .zip(&before)
                            .all(|(b, old)| b.text == *old)
                        {
                            event = None;
                        }
                        self.selection = Some(sel);
                    } else {
                        event = None;
                    }
                    None
                } else if doc
                    .blocks
                    .get(block)
                    .is_some_and(|b| b.kind.has_text() && !matches!(b.kind, BlockKind::Code { .. }))
                {
                    let before = doc.blocks[block].text.clone();
                    let (start, end) = marks::edit(
                        &mut doc.blocks[block].text,
                        start,
                        end,
                        MarkKind::Highlight,
                        None,
                        false,
                    );
                    let (start, end) = marks::edit(
                        &mut doc.blocks[block].text,
                        start,
                        end,
                        MarkKind::Underline,
                        None,
                        false,
                    );
                    if doc.blocks[block].text == before {
                        event = None;
                    }
                    self.pending_selection_end = Some(end);
                    Some(Caret { block, char: start })
                } else {
                    event = None;
                    None
                }
            }
            Op::Kind {
                block,
                kind,
                start,
                end,
            } => {
                let before = doc.blocks[block].kind.clone();
                ops::set_kind_in_place(&mut doc.blocks[block], kind);
                if doc.blocks[block].kind == before {
                    event = None;
                }
                self.pending_selection_end = Some(end);
                Some(Caret { block, char: start })
            }
            Op::FocusEnd => {
                if doc
                    .blocks
                    .last()
                    .is_some_and(|b| b.kind != BlockKind::Paragraph || !b.text.is_empty())
                {
                    doc.blocks.push(Block::paragraph(""));
                } else {
                    event = None;
                }
                Some(Caret {
                    block: doc.blocks.len() - 1,
                    char: 0,
                })
            }
            Op::Run {
                block,
                query,
                action,
            } => {
                self.menu_dismissed = None;
                // The space typed before `/` is part of the sentence. Removing
                // the query trims it when the command ends the block; a
                // citation is inline, so that space stays.
                let space_before_slash = {
                    let before = &doc.blocks[block].text[..query.start];
                    let cut = before.trim_end().len();
                    before[cut..].to_string()
                };
                let char = ops::remove_slash_query(&mut doc.blocks[block], &query);
                match action {
                    Action::SetBlock(kind) => Some(ops::set_block_kind(doc, block, kind)),
                    Action::InsertCitation => {
                        let char = crate::scripture_menu::restore_space(
                            &mut doc.blocks[block].text,
                            char,
                            &space_before_slash,
                        );
                        self.citation_form = Some(CitationForm::new(block, char));
                        None
                    }
                    Action::InsertScripture => {
                        let char = crate::scripture_menu::restore_space(
                            &mut doc.blocks[block].text,
                            char,
                            &space_before_slash,
                        );
                        self.scripture_picker =
                            Some(ScripturePicker::new(block, char, self.menu_anchor));
                        None
                    }
                    Action::InsertFileLink => {
                        let char = crate::scripture_menu::restore_space(
                            &mut doc.blocks[block].text,
                            char,
                            &space_before_slash,
                        );
                        self.file_picker = Some(FilePicker::new(block, char, self.menu_anchor));
                        None
                    }
                    other => {
                        event = Some(Event::Run(other));
                        Some(Caret { block, char })
                    }
                }
            }
        };
        doc.ensure_not_empty();
        if matches!(event, Some(Event::Changed)) {
            self.separate_change = true;
        }
        if event.is_some() {
            // Block indices may have shifted.
            self.galleys.clear();
            self.layouts.clear();
        }
        self.pending_caret = caret.map(|c| Caret {
            block: c.block.min(doc.blocks.len() - 1),
            ..c
        });
        self.rebind_outline(&outline_before);
        event
    }

    /// The next or previous block that isn't hidden by a collapsed outline item.
    fn neighbor_block(&self, from: usize, forward: bool) -> Option<usize> {
        let len = self.doc.blocks.len();
        let mut i = from;
        loop {
            if forward {
                if i + 1 >= len {
                    return None;
                }
                i += 1;
            } else if i == 0 {
                return None;
            } else {
                i -= 1;
            }
            if !self.hidden.get(i).copied().unwrap_or(false) {
                return Some(i);
            }
        }
    }

    /// The next or previous block that holds text and isn't folded away.
    fn neighbor_text(&self, from: usize, forward: bool) -> Option<usize> {
        let mut i = from;
        loop {
            i = self.neighbor_block(i, forward)?;
            if self.doc.blocks[i].kind.has_text() {
                return Some(i);
            }
        }
    }

    fn recompute_hidden(&mut self) {
        let n = self.doc.blocks.len();
        let mut hidden = vec![false; n];
        for i in 0..n {
            if hidden[i] || !self.collapsed.contains(&i) {
                continue;
            }
            if let Some(range) = self.doc.outline_child_range(i) {
                for child in range {
                    hidden[child] = true;
                }
            }
        }
        self.hidden = hidden;
    }

    /// Opens every collapsed ancestor of `index`.
    fn reveal_outline(&mut self, index: usize) {
        let mut i = 0;
        while i < index && i < self.doc.blocks.len() {
            if self.collapsed.contains(&i)
                && self
                    .doc
                    .outline_child_range(i)
                    .is_some_and(|range| range.contains(&index))
            {
                self.collapsed.remove(&i);
            }
            i += 1;
        }
    }

    /// First-line identity of each block, so a collapsed item can be found
    /// again after blocks are inserted or removed. Empty when nothing is folded.
    fn outline_keys(&self) -> Vec<String> {
        if self.collapsed.is_empty() {
            return Vec::new();
        }
        self.doc.blocks.iter().map(outline_key).collect()
    }

    fn rebind_outline(&mut self, before: &[String]) {
        if before.is_empty() || self.collapsed.is_empty() {
            return;
        }
        let after: Vec<String> = self.doc.blocks.iter().map(outline_key).collect();
        self.collapsed = rebind_collapsed(&self.collapsed, before, &after);
        self.collapsed
            .retain(|&i| self.doc.outline_child_range(i).is_some());
    }

    /// The margin number. When this paragraph has sub items, clicking it
    /// folds them. A chevron beside the number shows that they're hidden.
    fn paragraph_number(
        &mut self,
        ui: &mut Ui,
        i: usize,
        n: usize,
        right: f32,
        center_y: f32,
        palette: &Palette,
    ) {
        let collapsed = self.collapsed.contains(&i);
        let has_children = self.doc.outline_child_range(i).is_some();
        let color = if collapsed {
            palette.text
        } else {
            palette.subtle
        };
        let galley = ui
            .painter()
            .layout_no_wrap(n.to_string(), FontId::proportional(12.0), color);
        let text_pos = egui::pos2(right - galley.size().x, center_y - galley.size().y / 2.0);
        if has_children {
            let hit = Rect::from_min_max(
                egui::pos2(text_pos.x - 16.0, center_y - 11.0),
                egui::pos2(right + 2.0, center_y + 11.0),
            );
            let response = ui.interact(hit, self.block_id(i).with("outline"), Sense::click());
            let label = if collapsed {
                format!("Expand outline {i}")
            } else {
                format!("Collapse outline {i}")
            };
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
            });
            if response.hovered() {
                ui.painter().rect_filled(hit, 4.0, palette.hover);
            }
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.toggle_outline(ui, i);
            }
            // Right-pointing when the sub items are hidden; a faint one on
            // hover while they're open, so the control can be found.
            if collapsed || response.hovered() {
                crate::icons::chevron(
                    ui.painter(),
                    egui::pos2(text_pos.x - 8.0, center_y),
                    !self.collapsed.contains(&i),
                    if self.collapsed.contains(&i) {
                        palette.text
                    } else {
                        palette.faint
                    },
                );
            }
        }
        ui.painter().galley(text_pos, galley, color);
    }

    fn toggle_outline(&mut self, ui: &mut Ui, i: usize) {
        let Some(range) = self.doc.outline_child_range(i) else {
            return;
        };
        if self.collapsed.contains(&i) {
            self.collapsed.remove(&i);
            self.recompute_hidden();
            self.outline_dirty = true;
            return;
        }
        let parent_id = self.block_id(i);
        // A click outside the text field drops focus before this runs, so
        // the caret's block is the one that had it last frame.
        let caret_at = self.last_focus;
        let parent_focused = caret_at == Some(i) || ui.memory(|m| m.has_focus(parent_id));
        let child_focused = caret_at.is_some_and(|c| range.contains(&c))
            || range
                .clone()
                .any(|c| ui.memory(|m| m.has_focus(self.block_id(c))));
        self.collapsed.insert(i);
        if child_focused {
            for c in range {
                let id = self.block_id(c);
                if ui.memory(|m| m.has_focus(id)) {
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
            }
            // Keep typing on the item that stayed visible.
            self.pending_caret = Some(Caret {
                block: i,
                char: self.doc.blocks[i].text.chars().count(),
            });
            self.pending_selection_end = None;
        }
        self.recompute_hidden();
        self.outline_dirty = true;
        if parent_focused {
            ui.memory_mut(|m| m.request_focus(parent_id));
        }
    }
}

/// Maps collapsed block indices from `before` onto `after`.
///
/// Matching lines follow the longest common subsequence. An item whose own
/// text changed (typed in, or split with Enter) stays at its index when that
/// slot is still the same outline item.
fn rebind_collapsed(
    collapsed: &HashSet<usize>,
    before: &[String],
    after: &[String],
) -> HashSet<usize> {
    let map = lcs_index_map(before, after);
    let taken: HashSet<usize> = map.iter().flatten().copied().collect();
    let mut out = HashSet::new();
    for &i in collapsed {
        if let Some(j) = map.get(i).and_then(|j| *j) {
            out.insert(j);
            continue;
        }
        if !taken.contains(&i)
            && before
                .get(i)
                .zip(after.get(i))
                .is_some_and(|(old, new)| same_outline_item(old, new))
        {
            out.insert(i);
        }
    }
    out
}

fn same_outline_item(old: &str, new: &str) -> bool {
    let Some((old_kind, old_line)) = old.split_once(':') else {
        return false;
    };
    let Some((new_kind, new_line)) = new.split_once(':') else {
        return false;
    };
    if old_kind != new_kind {
        return false;
    }
    if old_line == new_line {
        return true;
    }
    let (short, long) = if old_line.len() <= new_line.len() {
        (old_line, new_line)
    } else {
        (new_line, old_line)
    };
    !short.is_empty() && long.starts_with(short)
}

/// For each index in `before`, the index of that same line in `after`.
fn lcs_index_map(before: &[String], after: &[String]) -> Vec<Option<usize>> {
    let n = before.len();
    let m = after.len();
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    let ix = |i: usize, j: usize| i * (m + 1) + j;
    for i in 1..=n {
        for j in 1..=m {
            dp[ix(i, j)] = if before[i - 1] == after[j - 1] {
                dp[ix(i - 1, j - 1)] + 1
            } else {
                dp[ix(i - 1, j)].max(dp[ix(i, j - 1)])
            };
        }
    }
    let mut map = vec![None; n];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if before[i - 1] == after[j - 1] {
            map[i - 1] = Some(j - 1);
            i -= 1;
            j -= 1;
        } else if dp[ix(i - 1, j)] >= dp[ix(i, j - 1)] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    map
}

/// How far an arrow key moves, from its modifiers: macOS uses Option for
/// words and Cmd for lines; Windows and Linux use Ctrl for words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Char,
    Word,
    /// To the start or end of the visual line.
    Line,
    /// Up or down one visual line.
    Row,
    /// To the start or end of the whole note.
    Document,
}

impl Motion {
    fn of(key: Key, m: Modifiers) -> Self {
        if matches!(key, Key::ArrowLeft | Key::ArrowRight) {
            if m.mac_cmd {
                Motion::Line
            } else if m.alt || m.ctrl {
                Motion::Word
            } else {
                Motion::Char
            }
        } else if m.command {
            Motion::Document
        } else {
            Motion::Row
        }
    }
}

/// Consumes the first arrow key pressed this frame that `wanted` accepts,
/// returning it with the modifiers held.
fn take_arrow(ui: &mut Ui, wanted: impl Fn(Key, Modifiers) -> bool) -> Option<(Key, Modifiers)> {
    let arrows = [
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
    ];
    ui.input_mut(|i| {
        let m = i.modifiers;
        let key = arrows
            .into_iter()
            .find(|&k| i.key_pressed(k) && wanted(k, m))?;
        i.consume_key(m, key);
        Some((key, m))
    })
}

fn consume(ui: &mut Ui, key: Key) -> bool {
    consume_mods(ui, Modifiers::NONE, key)
}

/// ⌘⇧H highlights in yellow. ⌘U underlines in blue. The same shortcut again
/// removes that annotation.
fn take_mark_shortcut(ui: &mut Ui) -> Option<(MarkKind, u32)> {
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    if consume_mods(ui, cmd_shift, Key::H) {
        return Some((MarkKind::Highlight, DEFAULT_HIGHLIGHT));
    }
    if consume_mods(ui, cmd, Key::U) {
        return Some((MarkKind::Underline, DEFAULT_UNDERLINE));
    }
    None
}

/// The first line of characters `from..to`, in screen coordinates.
fn span_rect(galley: &Galley, origin: Pos2, from: usize, to: usize) -> Option<Rect> {
    let mut row_start = 0;
    for row in &galley.rows {
        let row_end = row_start + row.char_count_excluding_newline().0;
        if from < row_end && to > row_start {
            let lo = from.max(row_start);
            let hi = to.min(row_end);
            let x0 = origin.x + row.pos.x + row.x_offset((lo - row_start).into());
            let x1 = origin.x + row.pos.x + row.x_offset((hi - row_start).into());
            if x1 > x0 {
                return Some(Rect::from_x_y_ranges(
                    x0..=x1,
                    origin.y + row.min_y()..=origin.y + row.max_y(),
                ));
            }
        }
        row_start += row.char_count_including_newline().0;
    }
    None
}

fn mark_colors(text: &str, start: usize, end: usize) -> (Option<u32>, Option<u32>) {
    (
        marks::active_color(text, start, end, MarkKind::Highlight),
        marks::active_color(text, start, end, MarkKind::Underline),
    )
}

fn consume_mods(ui: &mut Ui, modifiers: Modifiers, key: Key) -> bool {
    ui.input_mut(|i| i.consume_key(modifiers, key))
}

/// Hint shown in an empty block. An empty note always shows one, even
/// unfocused, so a new page never looks blank and broken.
fn placeholder(kind: &BlockKind, focused: bool, empty_note: bool, palette: &Palette) -> RichText {
    if empty_note && *kind == BlockKind::Paragraph {
        return RichText::new(EMPTY_NOTE_HINT)
            .size(16.0)
            .color(palette.faint);
    }
    let text = match kind {
        BlockKind::Heading(1) => "Heading 1",
        BlockKind::Heading(2) => "Heading 2",
        BlockKind::Heading(_) => "Heading 3",
        BlockKind::Bullet | BlockKind::Numbered => "List",
        BlockKind::Todo { .. } => "To-do",
        BlockKind::Quote => "Quote",
        BlockKind::Code { .. } => "",
        _ if focused => "Type '/' for commands",
        _ => "",
    };
    let size = match kind {
        BlockKind::Heading(1) => 30.0,
        BlockKind::Heading(2) => 24.0,
        BlockKind::Heading(_) => 19.0,
        _ => 16.0,
    };
    RichText::new(text).size(size).color(palette.subtle)
}

/// Where to draw the caret at `cursor`, in galley coordinates: from the top
/// of its line down to `text_height` (glyphs sit at the top of each line;
/// the extra line spacing is below them).
pub(crate) fn caret_rect(galley: &Galley, cursor: CCursor, text_height: f32) -> Rect {
    let line = galley.pos_from_cursor(cursor);
    let height = text_height.min(line.height());
    Rect::from_min_max(
        egui::pos2(line.min.x, line.min.y - 1.0),
        egui::pos2(line.min.x, line.min.y + height + 1.0),
    )
}

/// Highlight rectangles for characters `from..to` of a laid-out block.
/// `continues` marks the selection running on past the block's end, drawn as
/// a sliver after the last character (like a selected line break).
fn selection_rects(
    galley: &Galley,
    origin: Pos2,
    from: usize,
    to: usize,
    continues: bool,
    color: Color32,
) -> Vec<Shape> {
    find_bar::highlight_shapes(galley, origin, from, to, continues, color)
}

/// The URL of the link drawn under `pos` (relative to the galley), if any.
fn link_under(galley: &Galley, pos: egui::Vec2, text: &str) -> Option<String> {
    inline::link_at(text, char_under(galley, pos, text)?)
}

/// The byte offset of the character drawn under `pos` (relative to the
/// galley), if the pointer is on one.
fn char_under(galley: &Galley, pos: egui::Vec2, text: &str) -> Option<usize> {
    let nearest = galley.cursor_from_pos(pos).index.0;
    let len = text.chars().count();
    // The glyph under the pointer is on one side or the other of the nearest
    // cursor position, past any hidden (zero-width) markup like `[^`.
    let hit = (nearest.saturating_sub(1)..(nearest + 4).min(len)).find(|&c| {
        // Same row (glyphs on it can sit at different heights, like a
        // raised citation number), and between this glyph's edges.
        let row = galley.layout_from_cursor(CCursor::new(c)).row;
        if galley.layout_from_cursor(CCursor::new(c + 1)).row != row {
            return false;
        }
        let (a, b) = (
            galley.pos_from_cursor(CCursor::new(c)).min.x,
            galley.pos_from_cursor(CCursor::new(c + 1)).min.x,
        );
        let row = galley.rows[row].rect();
        (a..b).contains(&pos.x) && (row.min.y..=row.max.y).contains(&pos.y)
    })?;
    Some(ops::char_to_byte(text, hit))
}

#[cfg(test)]
mod outline_rebind_tests {
    use super::rebind_collapsed;
    use std::collections::HashSet;

    fn keys(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| (*s).to_string()).collect()
    }

    fn set(indices: &[usize]) -> HashSet<usize> {
        indices.iter().copied().collect()
    }

    #[test]
    fn inserting_a_block_before_a_folded_item_keeps_it_folded() {
        let before = keys(&["p:A", "p:\ta", "p:B", "p:\tb"]);
        let after = keys(&["p:A", "p:new", "p:\ta", "p:B", "p:\tb"]);
        assert_eq!(rebind_collapsed(&set(&[2]), &before, &after), set(&[3]));
    }

    #[test]
    fn splitting_a_folded_item_keeps_it_folded() {
        let before = keys(&["p:Parent long", "p:\tchild"]);
        let after = keys(&["p:Parent", "p:long", "p:\tchild"]);
        assert_eq!(rebind_collapsed(&set(&[0]), &before, &after), set(&[0]));
    }

    #[test]
    fn restore_collapsed_picks_the_matching_occurrence() {
        use scripture_study_core::{
            document::outline_key, settings::CollapsedOutline, Block, Document,
        };

        let doc = Document::new(vec![
            Block::paragraph("Parent"),
            Block::paragraph("\tChild"),
            Block::paragraph("Parent"),
            Block::paragraph("\tOther child"),
        ]);
        let mut editor = super::Editor::new("n", doc);
        let second = CollapsedOutline {
            key: outline_key(&Block::paragraph("Parent")),
            nth: 1,
        };
        editor.restore_collapsed(&[second.clone()]);
        assert!(editor.collapsed.contains(&2));
        assert!(!editor.collapsed.contains(&0));
        assert_eq!(editor.collapsed_outline(), vec![second]);
    }

    #[test]
    fn deleting_a_folded_item_drops_it() {
        let before = keys(&["p:Parent", "p:\tchild", "p:Sibling"]);
        let after = keys(&["p:\tchild", "p:Sibling"]);
        assert_eq!(rebind_collapsed(&set(&[0]), &before, &after), set(&[]));
    }
}

/// Copy or cut inside one block. A superscript's source is not in the
/// block's text, so the clipboard gets the selection plus its footnote
/// lines. `Some(None)` means the copy was handled and there is nothing
/// further to do.
fn copy_citation(
    editor: &Editor,
    ui: &mut Ui,
    block: usize,
    start: usize,
    end: usize,
) -> Option<Option<Op>> {
    if matches!(editor.doc.blocks.get(block)?.kind, BlockKind::Code { .. }) || start == end {
        return None;
    }
    let (from, to) = (start.min(end), start.max(end));
    let text = &editor.doc.blocks[block].text;
    let slice = &text[ops::char_to_byte(text, from)..ops::char_to_byte(text, to)];
    let markdown = citations::with_definitions(&editor.doc, slice);
    if markdown == slice {
        return None;
    }
    let cut = ui.input_mut(|input| {
        let n = input
            .events
            .iter()
            .position(|event| matches!(event, egui::Event::Copy | egui::Event::Cut))?;
        match input.events.remove(n) {
            egui::Event::Cut => Some(true),
            _ => Some(false),
        }
    })?;
    ui.ctx().copy_text(markdown);
    if cut {
        Some(Some(Op::InsertAt {
            block,
            start: from,
            end: to,
            text: String::new(),
        }))
    } else {
        Some(None)
    }
}

/// Takes a multi-line paste out of this frame's input. A single line is left
/// for the text field.
fn take_block_paste(ui: &mut Ui) -> Option<String> {
    ui.input_mut(|input| {
        let n = input
            .events
            .iter()
            .position(|e| matches!(e, egui::Event::Paste(text) if text.contains('\n')))?;
        match input.events.remove(n) {
            egui::Event::Paste(text) => Some(text),
            _ => None,
        }
    })
}

/// Takes a paste of a single URL out of this frame's input.
fn take_pasted_url(ui: &mut Ui) -> Option<String> {
    ui.input_mut(|input| {
        let n = input
            .events
            .iter()
            .position(|e| matches!(e, egui::Event::Paste(text) if links::as_url(text).is_some()))?;
        match input.events.remove(n) {
            egui::Event::Paste(text) => links::as_url(&text),
            _ => None,
        }
    })
}

type TextureCache = Arc<Mutex<HashMap<PathBuf, Option<egui::TextureHandle>>>>;

/// The picture at `path` as a texture, decoded once and then cached.
fn image_texture(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let cache = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<TextureCache>(Id::new("image-textures"))
            .clone()
    });
    let mut cache = cache.lock().ok()?;
    cache
        .entry(path.to_path_buf())
        .or_insert_with(|| {
            let image = image::open(path).ok()?.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            Some(ctx.load_texture(path.to_string_lossy(), pixels, egui::TextureOptions::LINEAR))
        })
        .clone()
}

/// Takes Tab (`Some(true)`, indent) or Shift+Tab (`Some(false)`, outdent).
fn take_tab(ui: &mut Ui) -> Option<bool> {
    if consume_mods(ui, Modifiers::SHIFT, Key::Tab) {
        Some(false)
    } else if consume(ui, Key::Tab) {
        Some(true)
    } else {
        None
    }
}
