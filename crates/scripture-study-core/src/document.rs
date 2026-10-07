//! The block-based document model and its Markdown representation.
//!
//! Notes are edited as a list of blocks (paragraphs, headings, list items, …)
//! and persisted as plain Markdown so they stay readable outside the app.

use crate::citations::{self, Citation};
use crate::inline;
use crate::properties::{self, Properties};

/// The kind of a block. Block-level Markdown syntax (`# `, `- `, `> `, …) is
/// captured here rather than in the block's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Text. Its lines can be indented one by one: each leading tab on a
    /// line is one level.
    Paragraph,
    /// Heading level, 1 through 6.
    Heading(u8),
    Bullet,
    Numbered,
    Todo {
        checked: bool,
    },
    Quote,
    Code {
        lang: String,
    },
    Divider,
    /// A picture, stored as `![alt](src)`. `src` is relative to the note's
    /// folder.
    Image {
        src: String,
        alt: String,
    },
}

impl BlockKind {
    /// Whether pressing Enter at the end of this block should continue with
    /// another block of the same kind.
    pub fn continues_on_enter(&self) -> bool {
        matches!(
            self,
            BlockKind::Bullet | BlockKind::Numbered | BlockKind::Todo { .. }
        )
    }

    /// Whether this block holds editable text.
    pub fn has_text(&self) -> bool {
        !matches!(self, BlockKind::Divider | BlockKind::Image { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Raw text including inline Markdown (`**bold**`, `*italic*`, …).
    /// May contain `\n` for soft line breaks.
    pub text: String,
    /// How many levels the block is indented (Tab). Stored in Markdown as
    /// one leading tab per level; for list items that nests the list.
    pub indent: u8,
}

/// The deepest a block can be indented.
pub const MAX_INDENT: u8 = 8;

/// An HTML comment used to persist an intentional empty editor row. A plain
/// Markdown blank line is only a paragraph separator, so it cannot survive a
/// Markdown round trip as an empty block.
const EMPTY_PARAGRAPH_MARKER: &str = "<!-- scripture-study:blank -->";

impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            indent: 0,
        }
    }

    pub fn indented(mut self, indent: u8) -> Self {
        self.indent = indent.min(MAX_INDENT);
        self
    }

    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::new(BlockKind::Paragraph, text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub properties: Properties,
    /// Sources cited in the text, in order (`[^1]: …` at the end of the file).
    pub citations: Vec<Citation>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            blocks: vec![Block::paragraph("")],
            properties: Properties::default(),
            citations: Vec::new(),
        }
    }
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        let mut doc = Self {
            blocks,
            properties: Properties::default(),
            citations: Vec::new(),
        };
        doc.ensure_not_empty();
        doc
    }

    /// A document always has at least one block to type into.
    pub fn ensure_not_empty(&mut self) {
        if self.blocks.is_empty() {
            self.blocks.push(Block::paragraph(""));
        }
    }

    pub fn is_blank(&self) -> bool {
        !self.properties.has_entries()
            && self.citations.is_empty()
            && self
                .blocks
                .iter()
                .all(|b| b.kind.has_text() && b.text.trim().is_empty())
    }

    /// The note's title: the plain text of its first non-empty block.
    pub fn title(&self) -> Option<String> {
        self.blocks
            .iter()
            .map(|b| inline::plain_text(b.text.lines().next().unwrap_or("")))
            .map(|t| t.trim().to_string())
            .find(|t| !t.is_empty())
    }

    /// Renames the note by rewriting the line its title comes from, keeping
    /// that block's kind. A note without a title gets one as its first line.
    pub fn set_title(&mut self, title: &str) {
        let title = title.trim();
        let titled = self.blocks.iter_mut().find(|b| {
            !inline::plain_text(b.text.lines().next().unwrap_or(""))
                .trim()
                .is_empty()
        });
        match titled {
            Some(block) => {
                let rest = block.text.find('\n').map(|n| block.text[n..].to_string());
                block.text = format!("{title}{}", rest.unwrap_or_default());
            }
            None => match self.blocks.first_mut() {
                Some(first) if first.kind.has_text() && first.text.is_empty() => {
                    first.text = title.to_string();
                }
                _ => self.blocks.insert(0, Block::paragraph(title)),
            },
        }
    }

    /// The 1-based number of each paragraph that has text, for numbering
    /// them in the margin. Other blocks (headings, lists, …) get `None`.
    ///
    /// A paragraph whose first line starts with tabs is indented under the
    /// text above it. It is numbered in that indent's own run, from 1, and
    /// the next paragraph at the outer indent keeps counting from where it
    /// left off.
    pub fn paragraph_numbers(&self) -> Vec<Option<usize>> {
        // Next number at each indent. A shallower paragraph drops the
        // deeper runs so the next indented paragraph starts at 1 again.
        let mut counts: Vec<usize> = Vec::new();
        self.blocks
            .iter()
            .map(|b| {
                if b.kind != BlockKind::Paragraph || b.text.trim().is_empty() {
                    return None;
                }
                let depth = paragraph_depth(&b.text);
                if counts.len() <= depth {
                    counts.resize(depth + 1, 0);
                } else {
                    counts.truncate(depth + 1);
                }
                counts[depth] += 1;
                Some(counts[depth])
            })
            .collect()
    }

    /// Blocks nested under `index`, when it has any.
    ///
    /// A following block is nested while its outline depth is greater.
    /// Empty paragraphs are gaps: they neither end the group nor count as a
    /// sub item by themselves. A paragraph's depth is the leading tabs on
    /// its first line; every other block uses its indent.
    pub fn outline_child_range(&self, index: usize) -> Option<std::ops::Range<usize>> {
        let parent = self.blocks.get(index).and_then(outline_depth)?;
        let mut end = index + 1;
        let mut nested = false;
        while end < self.blocks.len() {
            match outline_depth(&self.blocks[end]) {
                None => end += 1,
                Some(depth) if depth > parent => {
                    nested = true;
                    end += 1;
                }
                Some(_) => break,
            }
        }
        nested.then_some(index + 1..end)
    }

    /// 1-based position of the numbered item at `index` within its run.
    ///
    /// A run is the numbered items at one indent. Nested items are their own
    /// run, starting at 1, and do not count toward the parent. A shallower
    /// block, or anything else at this indent, ends the run.
    pub fn list_number(&self, index: usize) -> usize {
        let indent = self.blocks[index].indent;
        let mut count = 0;
        for block in self.blocks[..=index].iter().rev() {
            if block.kind == BlockKind::Numbered && block.indent == indent {
                count += 1;
            } else if block.indent > indent {
                continue;
            } else {
                break;
            }
        }
        count
    }

    pub fn to_markdown(&self) -> String {
        let mut body = self.body_markdown();
        // Citations go last, as Markdown footnotes.
        if !self.citations.is_empty() {
            if !body.is_empty() {
                body.push('\n');
            }
            for citation in &self.citations {
                body.push_str(&format!("[^{}]: {}\n", citation.id, citation.text));
            }
        }
        match self.properties.to_frontmatter() {
            Some(front) if body.is_empty() => front,
            Some(front) => {
                let mut out = front;
                out.push('\n');
                out.push_str(&body);
                out
            }
            None => body,
        }
    }

    fn body_markdown(&self) -> String {
        let mut out = String::new();
        let mut prev: Option<&BlockKind> = None;
        // Next number at each indent. A shallower item drops the deeper runs
        // so a nested list starts at 1 again.
        let mut numbers: Vec<usize> = Vec::new();

        for (index, block) in self.blocks.iter().enumerate() {
            // The final empty paragraph is editing scaffolding. Empty rows in
            // the middle of a note are intentional and need a marker because
            // a plain Markdown blank line is only a paragraph separator.
            if block.kind == BlockKind::Paragraph && block.text.trim().is_empty() {
                let has_content_before = self.blocks[..index]
                    .iter()
                    .any(|b| b.kind.has_text() && !b.text.trim().is_empty());
                let has_content_after = self.blocks[index + 1..]
                    .iter()
                    .any(|b| b.kind.has_text() && !b.text.trim().is_empty());
                if has_content_before && has_content_after {
                    if let Some(prev) = prev {
                        let same_list = prev.continues_on_enter()
                            && std::mem::discriminant(prev) == std::mem::discriminant(&block.kind);
                        out.push_str(if same_list { "\n" } else { "\n\n" });
                    }
                    out.push_str(EMPTY_PARAGRAPH_MARKER);
                    prev = Some(&block.kind);
                }
                continue;
            }
            if let Some(prev) = prev {
                let same_list = prev.continues_on_enter()
                    && block.kind.continues_on_enter()
                    && std::mem::discriminant(prev) == std::mem::discriminant(&block.kind);
                out.push_str(if same_list { "\n" } else { "\n\n" });
            }
            let start = out.len();
            let number = list_marker_number(&mut numbers, block);

            match &block.kind {
                BlockKind::Paragraph => out.push_str(&block.text),
                BlockKind::Heading(level) => {
                    out.push_str(&"#".repeat(*level as usize));
                    out.push(' ');
                    out.push_str(&block.text.replace('\n', " "));
                }
                BlockKind::Bullet => push_prefixed(&mut out, "- ", "  ", &block.text),
                BlockKind::Numbered => {
                    let prefix = format!("{number}. ");
                    let indent = " ".repeat(prefix.len());
                    push_prefixed(&mut out, &prefix, &indent, &block.text);
                }
                BlockKind::Todo { checked } => {
                    let prefix = if *checked { "- [x] " } else { "- [ ] " };
                    push_prefixed(&mut out, prefix, "      ", &block.text);
                }
                BlockKind::Quote => push_prefixed(&mut out, "> ", "> ", &block.text),
                BlockKind::Code { lang } => {
                    out.push_str("```");
                    out.push_str(lang);
                    out.push('\n');
                    out.push_str(&block.text);
                    out.push_str("\n```");
                }
                BlockKind::Divider => out.push_str("---"),
                BlockKind::Image { src, alt } => {
                    out.push_str(&format!("![{}]({})", alt.replace(['[', ']'], ""), src));
                }
            }
            if block.indent > 0 {
                let tabs = "\t".repeat(block.indent as usize);
                let lines: Vec<String> = out[start..]
                    .split('\n')
                    .map(|line| format!("{tabs}{line}"))
                    .collect();
                out.replace_range(start.., &lines.join("\n"));
            }
            prev = Some(&block.kind);
        }

        if !out.is_empty() {
            out.push('\n');
        }
        out
    }

    pub fn from_markdown(markdown: &str) -> Self {
        let (properties, markdown) = properties::split_frontmatter(markdown);
        let lines: Vec<&str> = markdown.lines().collect();
        let mut blocks: Vec<Block> = Vec::new();
        let mut cited = Vec::new();
        let mut i = 0;
        // The indent of the line that starts the next block.
        let mut pending: Option<(usize, u8)> = None;

        while i < lines.len() {
            if let Some((index, indent)) = pending.take() {
                if let Some(block) = blocks.get_mut(index) {
                    block.indent = indent;
                }
            }
            let line = lines[i];
            let trimmed = line.trim();

            if trimmed.is_empty() {
                i += 1;
                continue;
            }
            if trimmed == EMPTY_PARAGRAPH_MARKER {
                blocks.push(Block::paragraph(""));
                i += 1;
                continue;
            }
            let indent = indent_of(line, parse_list_item(line).is_some());
            pending = Some((blocks.len(), indent));

            if let Some(lang) = trimmed.strip_prefix("```") {
                let mut body = Vec::new();
                i += 1;
                let tabs = "\t".repeat(indent as usize);
                while i < lines.len() && lines[i].trim() != "```" {
                    body.push(lines[i].strip_prefix(tabs.as_str()).unwrap_or(lines[i]));
                    i += 1;
                }
                i += 1; // closing fence
                blocks.push(Block::new(
                    BlockKind::Code {
                        lang: lang.trim().to_string(),
                    },
                    body.join("\n"),
                ));
                continue;
            }

            if let Some(citation) = citations::parse_definition(trimmed) {
                cited.push(citation);
                pending = None;
                i += 1;
                continue;
            }

            if let Some(kind) = parse_image(trimmed) {
                blocks.push(Block::new(kind, ""));
                i += 1;
                continue;
            }

            if is_divider(trimmed) {
                blocks.push(Block::new(BlockKind::Divider, ""));
                i += 1;
                continue;
            }

            if let Some((level, text)) = parse_heading(trimmed) {
                blocks.push(Block::new(BlockKind::Heading(level), text));
                i += 1;
                continue;
            }

            if trimmed.starts_with('>') {
                let mut body = Vec::new();
                while i < lines.len() {
                    let Some(rest) = lines[i].trim().strip_prefix('>') else {
                        break;
                    };
                    body.push(rest.strip_prefix(' ').unwrap_or(rest));
                    i += 1;
                }
                blocks.push(Block::new(BlockKind::Quote, body.join("\n")));
                continue;
            }

            if let Some((kind, first, content_indent)) = parse_list_item(line) {
                let mut body = vec![first.to_string()];
                i += 1;
                // Continuation lines are indented to the item's content.
                while i < lines.len() {
                    let next = lines[i];
                    let indent = next.len() - next.trim_start().len();
                    if next.trim().is_empty()
                        || indent < content_indent.min(2)
                        // A nested item is a block of its own.
                        || parse_list_item(next).is_some()
                    {
                        break;
                    }
                    body.push(next.trim_start().to_string());
                    i += 1;
                }
                blocks.push(Block::new(kind, body.join("\n")));
                continue;
            }

            // A paragraph's leading tabs indent single lines, so they stay
            // in its text (see `BlockKind::Paragraph`) rather than indenting
            // the whole block.
            let line_text = |line: &str| line.trim_end().trim_start_matches(' ').to_string();
            pending = None;
            let mut body = vec![line_text(line)];
            i += 1;
            while i < lines.len() && starts_paragraph_continuation(lines[i]) {
                body.push(line_text(lines[i]));
                i += 1;
            }
            blocks.push(Block::paragraph(body.join("\n")));
        }

        if let Some((index, indent)) = pending {
            if let Some(block) = blocks.get_mut(index) {
                block.indent = indent;
            }
        }
        let mut doc = Self::new(blocks);
        doc.properties = properties;
        doc.citations = cited;
        doc
    }
}

/// Identity of a block for outline settings: its kind and first line.
///
/// Stored in `.scripture-study` so a folded item can be found again after
/// the note is reopened. The first line includes its leading tabs.
pub fn outline_key(block: &Block) -> String {
    let line = block.text.split('\n').next().unwrap_or("");
    let kind = match &block.kind {
        BlockKind::Paragraph => "p",
        BlockKind::Heading(level) => return format!("h{level}:{line}"),
        BlockKind::Bullet => "b",
        BlockKind::Numbered => "n",
        BlockKind::Todo { .. } => "t",
        BlockKind::Quote => "q",
        BlockKind::Code { .. } => "c",
        BlockKind::Divider => "d",
        BlockKind::Image { src, .. } => return format!("i:{src}"),
    };
    format!("{kind}:{line}")
}

/// Outline depth of a block that participates in nesting, or `None` when
/// the block is only a gap (an empty paragraph).
fn outline_depth(block: &Block) -> Option<usize> {
    match block.kind {
        BlockKind::Paragraph => {
            if block.text.trim().is_empty() {
                None
            } else {
                Some(paragraph_depth(&block.text))
            }
        }
        // A divider breaks an outline the way a heading at the left margin does.
        BlockKind::Divider => Some(0),
        _ => Some(block.indent as usize),
    }
}

/// How many leading tabs the paragraph's first line has.
fn paragraph_depth(text: &str) -> usize {
    text.split('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|&c| c == '\t')
        .count()
}

/// A line's indent level: one per leading tab, and one per 4 leading spaces
/// (2 for list items, which other editors often nest by 2).
fn indent_of(line: &str, list_item: bool) -> u8 {
    let (mut tabs, mut spaces) = (0, 0);
    for c in line.chars() {
        match c {
            '\t' => tabs += 1,
            ' ' => spaces += 1,
            _ => break,
        }
    }
    let per_level = if list_item { 2 } else { 4 };
    (tabs + spaces / per_level).min(MAX_INDENT as usize) as u8
}

/// The marker number for a numbered item, updating `numbers` for every block
/// so a later item at the same indent keeps counting.
fn list_marker_number(numbers: &mut Vec<usize>, block: &Block) -> usize {
    let depth = block.indent as usize;
    if block.kind != BlockKind::Numbered {
        if block.kind.continues_on_enter() {
            numbers.truncate(depth);
        } else {
            numbers.clear();
        }
        return 1;
    }
    numbers.truncate(depth + 1);
    if numbers.len() < depth + 1 {
        numbers.resize(depth + 1, 0);
    }
    numbers[depth] += 1;
    numbers[depth]
}

fn push_prefixed(out: &mut String, first: &str, rest: &str, text: &str) {
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        out.push_str(if n == 0 { first } else { rest });
        out.push_str(line);
    }
}

fn is_divider(trimmed: &str) -> bool {
    trimmed.len() >= 3
        && ["-", "*", "_"].iter().any(|c| {
            trimmed
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .all(|ch| ch.to_string() == *c)
        })
}

/// An image on a line of its own: `![alt](src)`.
fn parse_image(trimmed: &str) -> Option<BlockKind> {
    let rest = trimmed.strip_prefix("![")?.strip_suffix(')')?;
    let (alt, src) = rest.split_once("](")?;
    let src = src.trim();
    let src = src
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(src);
    if src.is_empty() || alt.contains(']') {
        return None;
    }
    Some(BlockKind::Image {
        src: src.to_string(),
        alt: alt.to_string(),
    })
}

fn parse_heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if rest.is_empty() {
        return Some((level as u8, ""));
    }
    rest.strip_prefix(' ')
        .map(|text| (level as u8, text.trim()))
}

/// Returns the list kind, the item's first line of text, and the column where
/// its content starts.
fn parse_list_item(line: &str) -> Option<(BlockKind, &str, usize)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();

    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            for (todo, checked) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if let Some(text) = rest.strip_prefix(todo) {
                    return Some((BlockKind::Todo { checked }, text, indent + 6));
                }
            }
            if let Some(checked) = match rest.trim_end() {
                "[ ]" => Some(false),
                "[x]" | "[X]" => Some(true),
                _ => None,
            } {
                return Some((BlockKind::Todo { checked }, "", indent + 6));
            }
            return Some((BlockKind::Bullet, rest, indent + 2));
        }
    }

    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &trimmed[digits..];
        for delim in [". ", ") "] {
            if let Some(text) = rest.strip_prefix(delim) {
                return Some((BlockKind::Numbered, text, indent + digits + 2));
            }
        }
    }
    None
}

fn starts_paragraph_continuation(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with("```")
        && !trimmed.starts_with('>')
        && !is_divider(trimmed)
        && parse_image(trimmed).is_none()
        && citations::parse_definition(trimmed).is_none()
        && parse_heading(trimmed).is_none()
        && parse_list_item(line).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(doc: &Document) -> Document {
        Document::from_markdown(&doc.to_markdown())
    }

    #[test]
    fn empty_document_has_one_block() {
        let doc = Document::from_markdown("");
        assert_eq!(doc.blocks, vec![Block::paragraph("")]);
        assert!(doc.is_blank());
        assert_eq!(doc.to_markdown(), "");
    }

    #[test]
    fn serializes_every_block_kind() {
        let doc = Document::new(vec![
            Block::new(BlockKind::Heading(1), "Groceries"),
            Block::paragraph("Things to **buy**."),
            Block::new(BlockKind::Bullet, "Milk"),
            Block::new(BlockKind::Bullet, "Eggs"),
            Block::new(BlockKind::Numbered, "First"),
            Block::new(BlockKind::Numbered, "Second"),
            Block::new(BlockKind::Todo { checked: false }, "Call mom"),
            Block::new(BlockKind::Todo { checked: true }, "Pay rent"),
            Block::new(BlockKind::Quote, "Be kind"),
            Block::new(BlockKind::Divider, ""),
            Block::new(
                BlockKind::Code {
                    lang: "rust".into(),
                },
                "fn main() {\n}",
            ),
        ]);
        let expected = "\
# Groceries

Things to **buy**.

- Milk
- Eggs

1. First
2. Second

- [ ] Call mom
- [x] Pay rent

> Be kind

---

```rust
fn main() {
}
```
";
        assert_eq!(doc.to_markdown(), expected);
        assert_eq!(roundtrip(&doc), doc);
    }

    #[test]
    fn multiline_blocks_roundtrip() {
        let doc = Document::new(vec![
            Block::paragraph("line one\nline two"),
            Block::new(BlockKind::Quote, "quoted\nmore"),
            Block::new(BlockKind::Bullet, "item\ncontinued"),
            Block::new(BlockKind::Numbered, "step\ndetail"),
        ]);
        assert_eq!(roundtrip(&doc), doc);
    }

    #[test]
    fn internal_empty_paragraphs_roundtrip() {
        let doc = Document::new(vec![
            Block::paragraph("a"),
            Block::paragraph(""),
            Block::paragraph("b"),
        ]);
        assert_eq!(
            doc.to_markdown(),
            "a\n\n<!-- scripture-study:blank -->\n\nb\n"
        );
        assert_eq!(roundtrip(&doc), doc);
    }

    #[test]
    fn trailing_empty_paragraph_is_editing_scaffolding() {
        let doc = Document::new(vec![Block::paragraph("a"), Block::paragraph("")]);
        assert_eq!(doc.to_markdown(), "a\n");
    }

    #[test]
    fn parses_common_markdown_variants() {
        let doc = Document::from_markdown("## Title\n* star\n+ plus\n3) paren\n***\n>no space\n");
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading(2),
                BlockKind::Bullet,
                BlockKind::Bullet,
                BlockKind::Numbered,
                BlockKind::Divider,
                BlockKind::Quote,
            ]
        );
        assert_eq!(doc.blocks[5].text, "no space");
    }

    #[test]
    fn title_uses_first_non_empty_block_without_markup() {
        let doc = Document::new(vec![
            Block::paragraph(""),
            Block::new(BlockKind::Heading(1), "My **big** idea"),
        ]);
        assert_eq!(doc.title().as_deref(), Some("My big idea"));
        assert_eq!(Document::default().title(), None);
    }

    #[test]
    fn list_numbers_restart_after_other_blocks() {
        let doc = Document::new(vec![
            Block::new(BlockKind::Numbered, "a"),
            Block::new(BlockKind::Numbered, "b"),
            Block::paragraph("break"),
            Block::new(BlockKind::Numbered, "c"),
        ]);
        assert_eq!(doc.list_number(1), 2);
        assert_eq!(doc.list_number(3), 1);
    }

    #[test]
    fn nested_numbered_lists_restart_at_each_indent() {
        let doc = Document::new(vec![
            Block::new(BlockKind::Numbered, "Prophecies"),
            Block::new(BlockKind::Numbered, "Jeremiah").indented(1),
            Block::new(BlockKind::Numbered, "Isaiah").indented(1),
            Block::new(BlockKind::Bullet, "aside").indented(2),
            Block::new(BlockKind::Numbered, "Zedekiah").indented(1),
            Block::new(BlockKind::Numbered, "Fulfillment"),
        ]);
        assert_eq!(
            doc.blocks
                .iter()
                .enumerate()
                .filter(|(i, _)| doc.blocks[*i].kind == BlockKind::Numbered)
                .map(|(i, _)| doc.list_number(i))
                .collect::<Vec<_>>(),
            vec![1, 1, 2, 3, 2]
        );
        let md = doc.to_markdown();
        assert_eq!(
            md,
            "\
1. Prophecies
	1. Jeremiah
	2. Isaiah

		- aside

	3. Zedekiah
2. Fulfillment
"
        );
        assert_eq!(roundtrip(&doc), doc);
    }

    #[test]
    fn images_roundtrip_on_their_own_line() {
        let md = "Before\n\n![A cat](.assets/cat.png)\n\nSee ![inline](x.png) here\n";
        let doc = Document::from_markdown(md);
        assert_eq!(
            doc.blocks[1].kind,
            BlockKind::Image {
                src: ".assets/cat.png".into(),
                alt: "A cat".into()
            }
        );
        assert_eq!(doc.blocks[2].kind, BlockKind::Paragraph);
        assert_eq!(doc.to_markdown(), md);
        assert!(!Document::new(vec![doc.blocks[1].clone()]).is_blank());
    }

    #[test]
    fn paragraphs_are_numbered_skipping_other_blocks() {
        let doc = Document::new(vec![
            Block::new(BlockKind::Heading(1), "Title"),
            Block::paragraph("One"),
            Block::paragraph("  "),
            Block::new(BlockKind::Bullet, "item"),
            Block::paragraph("Two"),
        ]);
        assert_eq!(
            doc.paragraph_numbers(),
            [None, Some(1), None, None, Some(2)]
        );
    }

    #[test]
    fn outline_child_range_covers_deeper_blocks_until_a_sibling() {
        let doc = Document::new(vec![
            Block::paragraph("Parent"),
            Block::paragraph("\tChild"),
            Block::paragraph("\t\tGrandchild"),
            Block::paragraph(""),
            Block::paragraph("\tOther child"),
            Block::paragraph("Sibling"),
            Block::new(BlockKind::Heading(1), "Next"),
        ]);
        assert_eq!(doc.outline_child_range(0), Some(1..5));
        // The blank line before the next item at this depth is part of the fold.
        assert_eq!(doc.outline_child_range(1), Some(2..4));
        assert_eq!(doc.outline_child_range(2), None);
        assert_eq!(doc.outline_child_range(4), None);
        assert_eq!(doc.outline_child_range(5), None);
        // Nothing is indented under the heading.
        assert_eq!(doc.outline_child_range(6), None);
    }

    #[test]
    fn outline_child_range_includes_an_indented_block_and_stops_at_a_peer() {
        let doc = Document::new(vec![
            Block::paragraph("Parent"),
            Block::new(BlockKind::Bullet, "nested").indented(1),
            Block::new(BlockKind::Bullet, "peer"),
            Block::paragraph("\tAfter the list"),
        ]);
        assert_eq!(doc.outline_child_range(0), Some(1..2));
        // The indented paragraph nests under the peer bullet, not the parent.
        assert_eq!(doc.outline_child_range(2), Some(3..4));
    }

    #[test]
    fn paragraph_numbers_restart_under_an_indent() {
        let doc = Document::new(vec![
            Block::paragraph("...that"),
            Block::paragraph("\tsdf"),
            Block::paragraph("...that sword."),
            Block::paragraph("\tfirst note"),
            Block::paragraph("\tsecond note"),
            Block::paragraph("\t\tdeeper"),
            Block::paragraph("after"),
        ]);
        assert_eq!(
            doc.paragraph_numbers(),
            [
                Some(1),
                Some(1),
                Some(2),
                Some(1),
                Some(2),
                Some(1),
                Some(3)
            ]
        );
    }

    #[test]
    fn indents_roundtrip_as_leading_tabs() {
        let doc = Document::new(vec![
            Block::paragraph("Top"),
            Block::paragraph("Plain\n\tindented line\n\t\tdeeper"),
            Block::new(BlockKind::Bullet, "parent"),
            Block::new(BlockKind::Bullet, "child").indented(1),
            Block::new(BlockKind::Bullet, "grandchild\nmore").indented(2),
            Block::new(BlockKind::Code { lang: "rs".into() }, "fn a() {\n\tb();\n}").indented(1),
            Block::new(
                BlockKind::Image {
                    src: "x.png".into(),
                    alt: String::new(),
                },
                "",
            )
            .indented(3),
            Block::new(BlockKind::Heading(2), "Deep").indented(1),
        ]);
        let md = doc.to_markdown();
        assert!(md.contains("Plain\n\tindented line\n\t\tdeeper"), "{md}");
        assert!(
            md.contains("- parent\n\t- child\n\t\t- grandchild\n\t\t  more"),
            "{md}"
        );
        assert_eq!(roundtrip(&doc), doc);

        // Other editors' space-nested lists.
        let spaced = Document::from_markdown("- a\n  - b\n    - c\n");
        let indents: Vec<u8> = spaced.blocks.iter().map(|b| b.indent).collect();
        assert_eq!(indents, [0, 1, 2]);
    }

    #[test]
    fn citations_roundtrip_as_footnotes() {
        let md = "Grace is sufficient[^1].\n\nAlso here[^2].\n\n[^1]: Smith, Jane. *Book*. 2012.\n[^2]: \u{201c}Page.\u{201d} *Site*.\n";
        let doc = Document::from_markdown(md);
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.citations.len(), 2);
        assert_eq!(doc.citations[0].text, "Smith, Jane. *Book*. 2012.");
        assert_eq!(doc.to_markdown(), md);
        assert_eq!(doc.title().as_deref(), Some("Grace is sufficient."));
    }

    #[test]
    fn set_title_rewrites_the_title_line_only() {
        let mut doc = Document::new(vec![
            Block::paragraph(" "),
            Block::new(BlockKind::Heading(1), "My **big** idea\nmore"),
            Block::paragraph("Body"),
        ]);
        doc.set_title("  Better idea ");
        assert_eq!(doc.title().as_deref(), Some("Better idea"));
        assert_eq!(doc.blocks[1].kind, BlockKind::Heading(1));
        assert_eq!(doc.blocks[1].text, "Better idea\nmore");
        assert_eq!(doc.blocks[2].text, "Body");

        let mut blank = Document::default();
        blank.set_title("Fresh");
        assert_eq!(blank.blocks, vec![Block::paragraph("Fresh")]);

        let mut divided = Document::new(vec![Block::new(BlockKind::Divider, "")]);
        divided.set_title("Top");
        assert_eq!(divided.blocks[0], Block::paragraph("Top"));
        assert_eq!(divided.blocks.len(), 2);
    }

    #[test]
    fn frontmatter_roundtrips_and_is_not_the_title() {
        let markdown = "\
---
tags: design
mentions: alex
status: draft
---

# Hello

Body
";
        let doc = Document::from_markdown(markdown);
        assert_eq!(doc.title().as_deref(), Some("Hello"));
        assert_eq!(doc.properties.tags, ["design"]);
        assert_eq!(doc.properties.mentions, ["alex"]);
        assert_eq!(roundtrip(&doc), doc);
        assert!(!doc.is_blank());

        let only_tags = Document::from_markdown("---\ntags: a\n---\n");
        assert!(!only_tags.is_blank());
        assert_eq!(only_tags.to_markdown(), "---\ntags: a\n---\n");
    }

    #[test]
    fn a_leading_divider_stays_a_divider() {
        let doc = Document::from_markdown("---\n\nhello\n");
        assert_eq!(
            doc.blocks
                .iter()
                .map(|b| b.kind.clone())
                .collect::<Vec<_>>(),
            vec![BlockKind::Divider, BlockKind::Paragraph]
        );
        assert!(doc.properties.tags.is_empty());
    }
}
