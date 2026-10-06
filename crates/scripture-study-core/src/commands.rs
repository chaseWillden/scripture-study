//! The slash menu: the commands it offers and how typing filters them.

use crate::document::BlockKind;
use crate::store::NoteMeta;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Turn the current block into this kind (or insert one below it).
    SetBlock(BlockKind),
    NewNote,
    OpenNote(String),
    DeleteNote,
    /// Cite a source at the caret.
    InsertCitation,
    /// Open the scripture picker; the chosen passage is cited by its reference.
    InsertScripture,
    /// Open the document picker; the chosen note is inserted as a link.
    InsertFileLink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub label: String,
    /// Short secondary text, e.g. the Markdown shortcut.
    pub hint: String,
    /// Extra words the command should match on.
    pub keywords: Vec<String>,
    pub action: Action,
}

impl Command {
    fn new(label: &str, hint: &str, keywords: &[&str], action: Action) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            action,
        }
    }
}

/// Built-in commands, in the order the menu shows them.
pub fn builtin() -> Vec<Command> {
    use BlockKind::*;
    vec![
        Command::new(
            "Text",
            "",
            &["paragraph", "plain"],
            Action::SetBlock(Paragraph),
        ),
        Command::new(
            "Heading 1",
            "#",
            &["h1", "title"],
            Action::SetBlock(Heading(1)),
        ),
        Command::new(
            "Heading 2",
            "##",
            &["h2", "subtitle"],
            Action::SetBlock(Heading(2)),
        ),
        Command::new("Heading 3", "###", &["h3"], Action::SetBlock(Heading(3))),
        Command::new(
            "Bulleted list",
            "-",
            &["ul", "unordered"],
            Action::SetBlock(Bullet),
        ),
        Command::new(
            "Numbered list",
            "1.",
            &["ol", "ordered"],
            Action::SetBlock(Numbered),
        ),
        Command::new(
            "To-do",
            "[]",
            &["todo", "task", "checkbox"],
            Action::SetBlock(Todo { checked: false }),
        ),
        Command::new("Quote", ">", &["blockquote"], Action::SetBlock(Quote)),
        Command::new(
            "Code",
            "```",
            &["snippet"],
            Action::SetBlock(Code {
                lang: String::new(),
            }),
        ),
        Command::new(
            "Divider",
            "---",
            &["hr", "line", "separator"],
            Action::SetBlock(Divider),
        ),
        Command::new(
            "Citation",
            "",
            &["cite", "footnote", "source", "reference"],
            Action::InsertCitation,
        ),
        Command::new(
            "Scripture",
            "",
            &["verse", "bible", "book of mormon", "lds"],
            Action::InsertScripture,
        ),
        Command::new(
            "File link",
            "",
            &["file-link", "document", "link"],
            Action::InsertFileLink,
        ),
        Command::new("New note", "", &["create", "page"], Action::NewNote),
        Command::new("Delete note", "", &["remove", "trash"], Action::DeleteNote),
    ]
}

/// Built-in commands followed by an "open" command for every other note.
pub fn with_notes(notes: &[NoteMeta], current: Option<&str>) -> Vec<Command> {
    let mut commands = builtin();
    commands.extend(
        notes
            .iter()
            .filter(|n| Some(n.id.as_str()) != current)
            .map(|n| Command {
                label: n.title.clone(),
                hint: "Open".into(),
                keywords: vec!["open".into(), "note".into()],
                action: Action::OpenNote(n.id.clone()),
            }),
    );
    commands
}

/// Commands matching `query`, best match first. An empty query keeps the
/// original order.
pub fn filter<'a>(commands: &'a [Command], query: &str) -> Vec<&'a Command> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return commands.iter().collect();
    }
    let mut scored: Vec<(i32, usize, &Command)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| score(&query, c).map(|s| (s, i, c)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

fn score(query: &str, command: &Command) -> Option<i32> {
    let label = command.label.to_lowercase();
    let mut best = match_text(query, &label);
    for keyword in &command.keywords {
        // Keyword hits rank just below equivalent label hits.
        best = best.max(match_text(query, keyword).map(|s| s - 5));
    }
    best
}

/// Scores how well `query` matches `text` (both lowercase).
fn match_text(query: &str, text: &str) -> Option<i32> {
    if text == query {
        return Some(100);
    }
    if text.starts_with(query) {
        return Some(80);
    }
    if text.split_whitespace().any(|w| w.starts_with(query)) {
        return Some(60);
    }
    if text.contains(query) {
        return Some(40);
    }
    // Subsequence: every query character appears in order ("hd1" → "heading 1").
    let mut chars = text.chars();
    let query = query.replace(' ', "");
    query.chars().all(|q| chars.any(|c| c == q)).then_some(20)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(query: &str) -> Vec<String> {
        filter(&builtin(), query)
            .into_iter()
            .map(|c| c.label.clone())
            .collect()
    }

    #[test]
    fn empty_query_shows_everything_in_order() {
        assert_eq!(
            labels(""),
            builtin().into_iter().map(|c| c.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn prefix_matches_rank_first() {
        assert_eq!(labels("head")[..3], ["Heading 1", "Heading 2", "Heading 3"]);
        assert_eq!(labels("heading 2")[0], "Heading 2");
    }

    #[test]
    fn keywords_and_subsequences_match() {
        assert_eq!(labels("h2")[0], "Heading 2");
        assert_eq!(labels("todo")[0], "To-do");
        assert_eq!(labels("file-link")[0], "File link");
        assert!(labels("hdg").contains(&"Heading 1".to_string()));
    }

    #[test]
    fn filters_out_non_matches() {
        assert!(labels("zzz").is_empty());
        assert!(!labels("quo").contains(&"Divider".to_string()));
    }

    #[test]
    fn notes_are_searchable_except_the_current_one() {
        let notes = vec![
            NoteMeta {
                id: "a".into(),
                title: "Groceries".into(),
                modified: std::time::SystemTime::UNIX_EPOCH,
                created: std::time::SystemTime::UNIX_EPOCH,
            },
            NoteMeta {
                id: "b".into(),
                title: "Trip plan".into(),
                modified: std::time::SystemTime::UNIX_EPOCH,
                created: std::time::SystemTime::UNIX_EPOCH,
            },
        ];
        let commands = with_notes(&notes, Some("b"));
        let found = filter(&commands, "groc");
        assert_eq!(found[0].action, Action::OpenNote("a".into()));
        assert!(filter(&commands, "trip").is_empty());
    }
}
