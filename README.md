# Scripture Study

A minimalist app for scripture study. It opens to a blank page. Type `/` to open the command
menu, and keep typing to filter it. Notes are stored as plain Markdown files and
shown as rich text.

## Quick start

```sh
make run      # launch the desktop app
make test     # run every test
make release  # optimized build at target/release/scripture-study
make cli      # symlink that binary onto PATH as `scripture-study`
make help     # list all targets
```

Requires a recent stable Rust toolchain (edition 2021, Rust 1.85+).

## Using it

| Do this | To get this |
| --- | --- |
| `/` | Open the command menu. Type to filter, use ↑/↓ to move, Enter or Tab to pick, Esc to close |
| `/new` or ⌘N | Create a new note |
| `/delete` or ⌘⇧⌫ | Delete the current note |
| ⌘K | Search all notes |
| ⌘F | Find in this note. Enter / ⇧Enter (or ⌘G / ⌘⇧G) for next / previous, **Fuzzy** to also match typos, Esc to close with the match selected |
| ⌘\ or ⌘⇧B | Show or hide the sidebar |
| ⌘S | Save now (notes also save on their own) |
| ⌘B / ⌘I / ⌘E / ⌘⇧X | Bold, italic, inline code, strikethrough. With no selection, the word under the caret is styled |
| ⌘⇧H / ⌘U | Highlight or underline. With no selection, the word under the caret is marked. Select text and the color bar appears above it |
| ⌘A | Select the whole note |
| Shift+↑/↓/←/→, Shift+click, or drag | Select text, including across paragraphs, lists, and headings. Typing or pasting replaces the selection; ⌘C/⌘X copy it as Markdown |
| ⌥⇧←/→ · ⌘⇧←/→ · ⌘⇧↑/↓ | Extend the selection by word · to the start/end of the line · to the start/end of the note |
| ⌘⌥1 / ⌘⌥2 / ⌘⌥3 | Heading 1, 2, or 3. ⌘⌥0 makes it plain text |
| ⌘⇧8 / ⌘⇧7 / ⌘⇧9 | Bulleted list, numbered list, to-do. ⌘↩ checks a to-do |
| ⌘⇧. | Quote. ⌘⌥8 makes a code block |
| `/` + a note's title | Open that note |

On Linux and Windows, Ctrl takes the place of ⌘.
| `# ` `## ` `### ` | Headings |
| `- ` or `* ` | Bulleted list |
| `1. ` | Numbered list |
| `[] ` | To-do (click the box to check it) |
| `> ` | Quote |
| ```` ``` ```` | Code block (press Enter on an empty last line to leave it) |
| `---` | Divider |
| `**bold**` `*italic*` `` `code` `` `~~strike~~` | Inline styles. Markers show while you edit a line and hide otherwise |
| Shift+Enter | Line break within a block |

### Browsing and search

The sidebar lists your notes, most recently edited first. Click one to open it,
or right-click it to delete it. When the sidebar is collapsed, an icon rail
stays on the left with buttons to show notes, search, and create a new note.

| Shortcut | Action |
| --- | --- |
| ⌘K | Search all notes by title and content. ↑/↓ to move, Enter to open, Esc to close |
| ⌘N | New note |
| ⌘\\ or ⌘⇧B | Show/hide the sidebar (also the button next to the window controls) |
| ⌘, | Open or close settings (also the gear at the bottom of the rail) |

On Windows and Linux, use Ctrl instead of ⌘.

### Folders

Click the folder icon in the left rail to organize notes. Folders can be
nested, and each one is a real directory inside the notes folder, so your
Markdown files stay organized on disk too.

- **New folder:** use the button next to "Folders", or right-click a folder
  to make a subfolder inside it.
- **Move a note:** drag it onto a folder (or onto the empty space below to
  move it to the top level), or right-click it and choose **Move to**.
- **Right-click a folder** to add a note to it, rename it, or delete it.
  Only empty folders can be deleted, so notes are never lost by accident.

### Conference talks

Click the microphone in the left rail to read and search general conference
talks, from April 1971 through the latest conference.

- **Download talks:** pick conferences (April or October), or click a year to
  pick both of its conferences, then choose **Download**. Every talk in them is
  fetched from churchofjesuschrist.org in the background, with progress shown
  in the sidebar. Downloads are kept, so they also work offline, in
  `.conference-talks` inside the platform data directory (for example,
  `~/Library/Application Support/scripture-study/.conference-talks` on macOS).
- **Search talks:** finds talks by title or speaker, then paragraphs that contain
  every word you type. Put the query in quotes to match an exact phrase.
  ↑/↓ to move, Enter to open. The talk opens on the matching paragraph.
- **Browse:** pick a downloaded conference in the sidebar to see its sessions and
  talks. Right-click it to remove the download.
- ⌘F finds within the open talk. Esc steps back.

### Settings and Google Drive sync

The gear at the bottom of the left rail (or ⌘,) opens **Settings**. Its
**Connectors** page holds the Google Drive connector. Turn it on, click
**Connect**, and approve access in your browser. From then on every note is
kept as a Google Doc in a Drive folder (**Scripture Study** unless you rename
it), with your note folders mirrored inside it.

- Sync is one way: from your notes to Drive. Edits made in Google Docs are
  overwritten the next time that note changes.
- A note syncs a few seconds after it's saved. **Sync now** syncs right away.
- The top of each note says whether Drive has its latest version: **Synced**,
  **Syncing**, or **Not synced** (hover for why).
- Deleting or moving a note moves its old Doc to the Drive trash.
- Frontmatter (tags, mentions, properties) and pictures are left out.
- The app asks only for the `drive.file` permission, so it can see the files
  it made and nothing else in your Drive.

Signing in needs a Google OAuth client. Builds made with
`SCRIPTURE_STUDY_GOOGLE_CLIENT_ID` (and `SCRIPTURE_STUDY_GOOGLE_CLIENT_SECRET`)
set, at build or run time, use that client. Otherwise the connector asks for
one: in Google Cloud Console, turn on the Google Drive API and create an
OAuth client of type **Desktop app**. The sign-in and sync records are kept in
`connectors.json` in the platform config directory (not in the notes folder),
readable only by you.

Notes save automatically half a second after you stop typing. Each note is one
`.md` file in `$SCRIPTURE_STUDY_DIR`. If that isn't set, notes go to the platform data
directory (`~/Library/Application Support/scripture-study` on macOS,
`~/.local/share/scripture-study` on Linux). The window title shows the note's title,
which is its first line of text. A new note left empty is discarded when you
switch away from it.

The top of a note is its properties: tags, mentions, the day it was created,
and when it was last updated. Type a tag or a person's name and press Enter
(commas add several at once); click one to remove it. **Add a property**
stores anything else. Those fields are saved as Markdown frontmatter:

```markdown
---
tags: design, ideas
mentions: alex
created: 2026-09-28T14:22:33Z
status: draft
---
```

## Layout

```
crates/
  scripture-study-core/   Platform-independent logic shared by every front end
    document.rs     Block model  <->  Markdown
    inline.rs       Inline Markdown spans (bold, italic, code, strike)
    editor.rs       Editing operations (split, merge, shortcuts, slash query)
    commands.rs     Slash menu commands + fuzzy filtering
    search.rs       Full-text note search, snippets, relative times
    talks.rs        Conference list, study-page HTML → talk text, talk search
    drive_sync.rs   What to upload, update, or trash to mirror notes in a cloud folder
    selection.rs    Selections spanning blocks: copy, delete, replace, paste
    folders.rs      Folder tree for organizing notes
    store.rs        NoteStore trait + filesystem implementation (with folders)
apps/
  desktop/        egui/eframe desktop app (binary: `scripture-study`)
    src/app.rs      Note lifecycle, autosave, window layout, shortcuts
    src/editor.rs   Block editor widget + slash menu
    src/sidebar.rs  Note list, search, new note, and the icon rail
    src/talks.rs    Conference talks page: picker, search, sessions, talk reader
    src/talk_library.rs   Saved talks and the background download thread
    src/settings_page.rs  Settings sections and the Connectors page
    src/google_drive.rs   Google sign-in, the Drive API, and the background sync thread
    src/icons.rs    Line icons (painted, no icon font)
    src/theme.rs    Fonts, colors, Markdown → rich text layout
    src/app/tests.rs  Headless UI tests (egui_kittest)
  web/            Planned: WASM front end
  mobile/         Planned: iOS/Android via UniFFI
```

Keep new logic in `scripture-study-core` whenever it doesn't depend on a UI. That way
the web and mobile apps get it for free. `NoteStore` is the extension point for
other storage backends (browser storage, sync).

## Testing

`make test` runs:

- **Core unit tests**: Markdown round-trips, inline parsing, editing
  operations, command filtering, and storage.
- **Headless UI tests**: these drive the real desktop app with simulated
  keystrokes and clicks. They type `/`, filter, pick commands, and check the
  Markdown written to disk.

`make snapshots` renders PNG screenshots from the UI tests into
`target/snapshots`.

## Fonts

The desktop app bundles [Inter](https://rsms.me/inter/) (SIL Open Font License,
see `apps/desktop/assets/fonts/OFL.txt`).
