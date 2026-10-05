# Web app (planned)

Will reuse `scripture-study-core` for the document model, Markdown storage, and slash
commands. Two likely routes:

- Compile the desktop egui UI to WASM with `eframe`'s web runner, and add a
  `NoteStore` implementation backed by browser storage or a sync API.
- A native web UI that calls `scripture-study-core` through `wasm-bindgen`.

When work starts, add the crate to `members` in the root `Cargo.toml`.
