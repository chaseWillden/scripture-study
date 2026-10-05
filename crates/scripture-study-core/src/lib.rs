//! Platform-agnostic core of Scripture Study.
//!
//! Everything here is plain Rust with no UI or platform dependencies so the
//! desktop, web (WASM), and mobile front ends can share it.

pub mod citations;
pub mod commands;
pub mod document;
pub mod editor;
pub mod find;
pub mod folders;
pub mod history;
pub mod inline;
pub mod links;
pub mod properties;
pub mod scriptures;
pub mod search;
pub mod selection;
pub mod store;
mod time;

pub use document::{Block, BlockKind, Document};
pub use properties::Properties;
pub use store::{FsStore, NoteMeta, NoteStore};
