//! Keyboard shortcuts shared by the window and the sidebar.
//!
//! `Modifiers::COMMAND` is ⌘ on macOS and Ctrl elsewhere. Block and inline
//! shortcuts live next to the editor, which is the only place that handles them.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

const fn command(shift: bool, alt: bool) -> Modifiers {
    Modifiers {
        alt,
        ctrl: false,
        shift,
        mac_cmd: false,
        command: true,
    }
}

pub const TOGGLE_SIDEBAR: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Backslash);
/// Same toggle as [`TOGGLE_SIDEBAR`]. Must be consumed before ⌘B, which bolds:
/// egui treats an extra Shift as a match for the unmodified shortcut.
pub const TOGGLE_SIDEBAR_SHIFT_B: KeyboardShortcut =
    KeyboardShortcut::new(command(true, false), Key::B);
pub const SEARCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::K);
pub const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
pub const NEW_NOTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
pub const OPEN_FOLDER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
pub const DELETE_NOTE: KeyboardShortcut =
    KeyboardShortcut::new(command(true, false), Key::Backspace);
/// Check before [`RENAME`], which also matches with Option held.
pub const REVEAL: KeyboardShortcut = KeyboardShortcut::new(command(false, true), Key::R);
pub const RENAME: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
/// Check before [`NEW_NOTE`], which also matches with Shift held.
pub const NEW_FOLDER: KeyboardShortcut = KeyboardShortcut::new(command(true, false), Key::N);
/// The usual place for an app's settings on every platform.
pub const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
