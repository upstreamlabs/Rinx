//! Versioned article documents. Display, editing and publishing share this model.
use std::ops::Range;
use serde::{Deserialize, Serialize};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_BODY: usize = 512 * 1024;
pub const MAX_BLOCKS: usize = 8192;
pub const MAX_IMAGES: usize = 128;
pub fn new_id() -> String {
    { use rand::RngCore; let mut bytes = [0u8; 16]; rand::thread_rng().fill_bytes(&mut bytes); bytes.iter().map(|byte| format!("{byte:02x}")).collect() }
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub use makepad_markdown::escape;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Classic,
    Paper,
    Ocean,
    Ink,
    Magazine,
    NewYorkTimes,
    FinancialTimes,
    Minimal,
    Tech,
    Longform,
    Elegant,
    DeepReading,
}
impl Theme {
    pub const ALL: [Self; 12] = [
        Self::Classic, Self::Paper, Self::Ocean, Self::Ink,
        Self::Magazine, Self::NewYorkTimes, Self::FinancialTimes, Self::Minimal,
        Self::Tech, Self::Longform, Self::Elegant, Self::DeepReading,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic green",
            Self::Paper => "Warm paper",
            Self::Ocean => "Ocean blue",
            Self::Ink => "Minimal ink",
            Self::Magazine => "Magazine",
            Self::NewYorkTimes => "New York Times",
            Self::FinancialTimes => "Financial Times",
            Self::Minimal => "Minimal",
            Self::Tech => "Tech",
            Self::Longform => "Longform",
            Self::Elegant => "Elegant",
            Self::DeepReading => "Deep reading",
        }
    }
    /// (paper, ink, accent) colors.
    pub fn colors(self) -> (u32, u32, u32) {
        match self {
            Self::Classic => (0xffffff, 0x191919, 0x07a858),
            Self::Paper => (0xfaf5eb, 0x443d32, 0x987550),
            Self::Ocean => (0xf4f9fc, 0x263c4b, 0x398cba),
            Self::Ink => (0xffffff, 0x202020, 0x353535),
            Self::Magazine => (0xffffff, 0x1a1a1a, 0xc0392b),
            Self::NewYorkTimes => (0xffffff, 0x121212, 0x326891),
            Self::FinancialTimes => (0xfff1e5, 0x33302e, 0x990f3d),
            Self::Minimal => (0xffffff, 0x333333, 0x888888),
            Self::Tech => (0xf7f9fc, 0x1f2937, 0x2563eb),
            Self::Longform => (0xfdfdfb, 0x2b2b2b, 0x7a5c3e),
            Self::Elegant => (0xfbf8f3, 0x3a3230, 0xb08d57),
            Self::DeepReading => (0xf8f6f1, 0x2d2a26, 0x5b6b4f),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    #[default]
    Paragraph,
    Heading2,
    Heading3,
    Quote,
    Bullet,
    Numbered,
    Image,
    Divider,
    /// A complete Markdown structure edited as source and rendered in previews.
    Markdown,
    /// Imported HTML is retained verbatim; rendering always sanitizes it.
    Html,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub alt: String,
    #[serde(default = "full_width")]
    pub width: u8,
}
fn full_width() -> u8 {
    100
}
impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            kind,
            text: text.into(),
            marks: vec![],
            asset: None,
            caption: String::new(),
            alt: String::new(),
            width: 100,
        }
    }
    pub fn flags_at(&self, pos: usize) -> (bool, bool, Option<String>) {
        let mut flags = (false, false, None);
        for m in &self.marks {
            if pos >= m.start && pos < m.end {
                flags.0 |= m.bold;
                flags.1 |= m.italic;
                if m.link.is_some() {
                    flags.2 = m.link.clone()
                }
            }
        }
        flags
    }
    /// Selection boundaries are grapheme boundaries, not character or byte counts.
    pub fn format(
        &mut self,
        selection: Range<usize>,
        bold: Option<bool>,
        italic: Option<bool>,
        link: Option<Option<String>>,
    ) -> Result<(), String> {
        if matches!(self.kind, BlockKind::Markdown | BlockKind::Html) {
            return Err("Edit this block's formatting in its source.".into());
        }
        if selection.start > selection.end
            || selection.end > self.text.len()
            || !self.text.is_char_boundary(selection.start)
            || !self.text.is_char_boundary(selection.end)
        {
            return Err("Invalid text selection".into());
        }
        if let Some(Some(url)) = &link {
            validate_link(url)?;
        }
        let mut marks = Vec::new();
        for (i, g) in self.text.grapheme_indices(true) {
            let (mut b, mut it, mut l) = self.flags_at(i);
            if i < selection.end && i + g.len() > selection.start {
                if let Some(v) = bold {
                    b = v
                }
                if let Some(v) = italic {
                    it = v
                }
                if let Some(v) = &link {
                    l = v.clone()
                }
            }
            if b || it || l.is_some() {
                push_mark(
                    &mut marks,
                    Mark {
                        start: i,
                        end: i + g.len(),
                        bold: b,
                        italic: it,
                        link: l,
                    },
                );
            }
        }
        self.marks = marks;
        Ok(())
    }
    /// Preserve formatting around an IME/native input edit, including Unicode.
    pub fn replace_text(&mut self, next: String) {
        if self.text == next {
            return;
        }
        let mut prefix = 0;
        for (a, b) in self.text.chars().zip(next.chars()) {
            if a != b {
                break;
            }
            prefix += a.len_utf8();
        }
        let old_tail = &self.text[prefix..];
        let new_tail = &next[prefix..];
        let mut suffix = 0;
        for (a, b) in old_tail.chars().rev().zip(new_tail.chars().rev()) {
            if a != b {
                break;
            }
            suffix += a.len_utf8();
        }
        let old_end = self.text.len() - suffix;
        let new_end = next.len() - suffix;
        let inherited = self.flags_at(if prefix == 0 {
            0
        } else {
            self.text[..prefix]
                .char_indices()
                .last()
                .map(|v| v.0)
                .unwrap_or(0)
        });
        let mut marks = Vec::new();
        for (i, g) in next.grapheme_indices(true) {
            let flags = if i < prefix {
                self.flags_at(i)
            } else if i >= new_end {
                self.flags_at(old_end + i - new_end)
            } else {
                inherited.clone()
            };
            if flags.0 || flags.1 || flags.2.is_some() {
                push_mark(
                    &mut marks,
                    Mark {
                        start: i,
                        end: i + g.len(),
                        bold: flags.0,
                        italic: flags.1,
                        link: flags.2,
                    },
                );
            }
        }
        self.text = next;
        self.marks = marks;
    }
    pub fn inline_html(&self) -> String {
        let mut result = String::new();
        let mut start = 0;
        while start < self.text.len() {
            let flags = self.flags_at(start);
            let mut end = self.text.len();
            for (offset, _) in self.text[start..].char_indices().skip(1) {
                if self.flags_at(start + offset) != flags {
                    end = start + offset;
                    break;
                }
            }
            let mut text = escape(&self.text[start..end]).replace('\n', "<br>");
            if flags.0 {
                text = format!("<strong>{text}</strong>")
            }
            if flags.1 {
                text = format!("<em>{text}</em>")
            }
            if let Some(url) = flags.2 {
                text = format!("<a href=\"{}\">{text}</a>", escape(&url))
            }
            result.push_str(&text);
            start = end;
        }
        result
    }
    pub fn html(&self) -> String {
        let content = self.inline_html();
        match self.kind {
            BlockKind::Markdown => crate::markup::markdown_html(&self.text),
            BlockKind::Html => crate::markup::html_fragment(&self.text),
            BlockKind::Paragraph => format!("<p>{content}</p>"),
            BlockKind::Heading2 => format!("<h2>{content}</h2>"),
            BlockKind::Heading3 => format!("<h3>{content}</h3>"),
            BlockKind::Quote => format!("<blockquote>{content}</blockquote>"),
            BlockKind::Bullet => format!("<ul><li>{content}</li></ul>"),
            BlockKind::Numbered => format!("<ol><li>{content}</li></ol>"),
            BlockKind::Divider => "<hr>".into(),
            BlockKind::Image => format!(
                "<p>[{}]</p>",
                escape(if self.caption.is_empty() {
                    &self.alt
                } else {
                    &self.caption
                })
            ),
        }
    }
}
fn push_mark(marks: &mut Vec<Mark>, mark: Mark) {
    if let Some(last) = marks.last_mut() {
        if last.end == mark.start
            && last.bold == mark.bold
            && last.italic == mark.italic
            && last.link == mark.link
        {
            last.end = mark.end;
            return;
        }
    }
    marks.push(mark);
}
pub fn validate_link(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|_| "Use a complete HTTPS link.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.len() > 2048
    {
        return Err("Use a complete HTTPS link.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cover {
    pub asset: String,
    pub focal_x: u16,
    pub focal_y: u16,
    pub show_in_article: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub theme: Theme,
    pub large_type: bool,
    pub compact: bool,
    pub cover: Option<Cover>,
    pub blocks: Vec<Block>,
    pub modified: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference_definitions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_source: Option<ImportedSource>,
    /// Original relative image references mapped to imported article assets.
    /// Filesystem paths stay in host-only local metadata, never publications.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub resource_bindings: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedSource {
    pub text: String,
    pub blocks_hash: String,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            schema: 2,
            id: new_id(),
            title: String::new(),
            author: String::new(),
            summary: String::new(),
            theme: Theme::default(),
            large_type: false,
            compact: false,
            cover: None,
            blocks: vec![Block::new(BlockKind::Paragraph, "")],
            modified: now(),
            reference_definitions: String::new(),
            imported_source: None,
            resource_bindings: Default::default(),
        }
    }
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 2
            || self.reference_definitions.len() > MAX_BODY
            || self.imported_source.as_ref().is_some_and(|s| s.text.len() > MAX_BODY || s.blocks_hash.len() != 64)
            || !valid_id(&self.id)
            || self.title.chars().count() > 120
            || self.title.chars().any(char::is_control)
            || self.author.chars().count() > 80
            || self.summary.chars().count() > 240
            || self.blocks.len() > MAX_BLOCKS
            || self
                .blocks
                .iter()
                .map(|b| b.text.len() + b.caption.len() + b.alt.len())
                .sum::<usize>()
                > MAX_BODY
        {
            return Err("Article exceeds its size limits.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for b in &self.blocks {
            if !valid_id(&b.id)
                || !ids.insert(&b.id)
                || ![50, 75, 100].contains(&b.width)
                || b.asset.as_ref().is_some_and(|a| !valid_id(a))
                || b.caption.chars().count() > 240
                || b.alt.chars().count() > 240
            {
                return Err("Invalid article block".into());
            }
            if b.marks.len() > 2048 || (b.kind == BlockKind::Image && b.asset.is_none()) {
                return Err("Invalid article block".into());
            }
            for m in &b.marks {
                if m.start >= m.end
                    || m.end > b.text.len()
                    || !b.text.is_char_boundary(m.start)
                    || !b.text.is_char_boundary(m.end)
                {
                    return Err("Invalid article formatting".into());
                }
                if let Some(url) = &m.link {
                    validate_link(url)?
                }
            }
        }
        if self.asset_ids().len() > MAX_IMAGES || self.resource_bindings.iter().any(|(url,id)|url.len()>4096 || !valid_id(id)) {
            return Err("Use at most 128 images per article.".into());
        }
        if let Some(c) = &self.cover {
            if !valid_id(&c.asset) || c.focal_x > 1000 || c.focal_y > 1000 {
                return Err("Invalid cover crop".into());
            }
        }
        Ok(())
    }
    pub fn ready(&self) -> Result<(), String> {
        self.validate()?;
        if self.title.trim().is_empty()
            || !self
                .blocks
                .iter()
                .any(|b| !b.text.trim().is_empty() || b.kind == BlockKind::Image)
        {
            return Err("Add a title and some text first.".into());
        }
        Ok(())
    }
    pub fn asset_ids(&self) -> Vec<String> {
        let mut ids = std::collections::BTreeSet::new();
        ids.extend(self.resource_bindings.values().cloned());
        if let Some(c) = &self.cover {
            ids.insert(c.asset.clone());
        }
        for b in &self.blocks {
            if let Some(a) = &b.asset {
                ids.insert(a.clone());
            }
        }
        ids.into_iter().collect()
    }
    pub fn stats(&self) -> (usize, usize, usize) {
        let chars = self
            .blocks
            .iter()
            .map(|b| b.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum::<usize>();
        (chars, (chars / 350 + 1).max(1), self.asset_ids().len() + crate::render::image_requests(self).len())
    }
    pub fn html(&self) -> String {
        format!(
            "<h1>{}</h1><p>{}</p>{}",
            escape(&self.title),
            escape(&self.author),
            self.blocks.iter().map(|b| self.block_html(b)).collect::<String>()
        )
    }
    pub fn block_html(&self, block: &Block) -> String {
        if block.kind == BlockKind::Markdown && !self.reference_definitions.is_empty() {
            crate::markup::markdown_html(&format!("{}\n\n{}", block.text, self.reference_definitions))
        } else {
            block.html()
        }
    }
    pub fn block_html_with_math(&self, block: &Block, render: impl FnMut(&crate::math::Formula) -> String) -> String {
        if block.kind == BlockKind::Markdown {
            crate::math::markdown_html(&format!("{}\n\n{}", block.text, self.reference_definitions), render)
        } else {
            block.html()
        }
    }
    pub fn is_html_source(&self) -> bool {
        self.blocks.len() == 1 && self.blocks[0].kind == BlockKind::Html
    }
    pub fn block_html_with_renderer(&self, block: &Block, renderer: &mut dyn crate::render::Renderer) -> String {
        match block.kind {
            BlockKind::Markdown => crate::math::markdown_html_with_renderer(&format!("{}\n\n{}", block.text, self.reference_definitions), renderer),
            BlockKind::Html => crate::markup::html_fragment_with_renderer(&block.text, renderer),
            _ => crate::markup::html_fragment_with_renderer(&block.html(), renderer),
        }
    }
    fn blocks_hash(&self) -> String {
        blake3::hash(&serde_json::to_vec(&self.blocks).expect("article blocks serialize")).to_hex().to_string()
    }
    /// Keep the original spelling until the body is edited. Metadata-only edits
    /// do not discard it; the block digest prevents stale source from winning.
    pub fn retain_source(&mut self, source: &str) {
        self.imported_source = Some(ImportedSource { text: source.into(), blocks_hash: self.blocks_hash() });
    }
    pub fn from_markdown(title: &str, markdown: &str) -> Result<Self, String> {
        crate::markup::import_markdown(title, markdown)
    }
    pub fn from_html(title: &str, html: &str) -> Result<Self, String> {
        if html.len() > MAX_BODY { return Err("Article exceeds its size limits.".into()); }
        let mut doc = Self { title: title.into(), blocks: vec![Block::new(BlockKind::Html, html)], ..Self::default() };
        doc.retain_source(html);
        doc.validate()?;
        Ok(doc)
    }
    pub(crate) fn from_visual_markdown(title: &str, markdown: &str) -> Result<Self, String> {
        if markdown.len() > MAX_BODY {
            return Err("Article exceeds its size limits.".into());
        }
        let mut doc = Self {
            title: title.into(),
            blocks: Vec::new(),
            ..Self::default()
        };
        let mut block: Option<Block> = None;
        let mut bold = 0;
        let mut italic = 0;
        let mut link = None;
        let mut list = false;
        let mut ordered = false;
        let mut quote = false;
        for event in Parser::new(markdown) {
            match event {
                Event::Start(Tag::Paragraph)=>{if block.is_none(){block=Some(Block::new(if quote {BlockKind::Quote} else {BlockKind::Paragraph},""));}},
                Event::Start(Tag::Heading{level,..})=>block=Some(Block::new(if (level as u8)<=2 {BlockKind::Heading2}else{BlockKind::Heading3},"")),
                Event::Start(Tag::BlockQuote(_))=>quote=true,
                Event::Start(Tag::List(start))=>{if list {return Err("Nested lists are not supported in visual mode. Your source has been kept.".into())}list=true;ordered=start.is_some();},
                Event::Start(Tag::Item)=>block=Some(Block::new(if ordered {BlockKind::Numbered}else{BlockKind::Bullet},"")),
                Event::Start(Tag::Strong)=>bold+=1,Event::End(TagEnd::Strong)=>bold-=1,
                Event::Start(Tag::Emphasis)=>italic+=1,Event::End(TagEnd::Emphasis)=>italic-=1,
                Event::Start(Tag::Link{dest_url,..})=>{validate_link(&dest_url)?;link=Some(dest_url.to_string());},Event::End(TagEnd::Link)=>link=None,
                Event::Start(Tag::Image{dest_url,..})=>{
                    if let Some(b)=block.take() {if !b.text.is_empty(){doc.blocks.push(b)}}
                    let id=dest_url.strip_prefix("asset:").filter(|v|valid_id(v)).ok_or("Insert images using the image picker.")?;
                    let mut b=Block::new(BlockKind::Image,""); b.asset=Some(id.into());block=Some(b);
                },
                Event::End(TagEnd::Image)=>{if let Some(mut b)=block.take(){b.alt=b.text.clone();b.text.clear();doc.blocks.push(b)}},
                Event::Text(text)=>{let b=block.get_or_insert_with(||Block::new(BlockKind::Paragraph,""));let start=b.text.len();b.text.push_str(&text);if start<b.text.len()&&(bold>0||italic>0||link.is_some()) {push_mark(&mut b.marks,Mark{start,end:b.text.len(),bold:bold>0,italic:italic>0,link:link.clone()});}},
                Event::SoftBreak|Event::HardBreak=>{if let Some(b)=&mut block{b.text.push('\n')}},
                Event::Rule=>doc.blocks.push(Block::new(BlockKind::Divider,"")),
                Event::End(TagEnd::Paragraph) if !list=>{if let Some(b)=block.take(){doc.blocks.push(b)}},
                Event::End(TagEnd::Heading(_))|Event::End(TagEnd::Item)=>{if let Some(b)=block.take(){doc.blocks.push(b)}},
                Event::End(TagEnd::List(_))=>list=false,Event::End(TagEnd::BlockQuote(_))=>quote=false,
                Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) | Event::Start(Tag::CodeBlock(_))=>return Err("This Markdown contains unsupported HTML or code blocks. Your source has been kept.".into()),
                _=>(),
            }
        }
        if let Some(b) = block {
            doc.blocks.push(b)
        }
        if doc.blocks.is_empty() {
            doc.blocks.push(Block::new(BlockKind::Paragraph, ""))
        }
        doc.validate()?;
        Ok(doc)
    }
    pub fn markdown(&self) -> String {
        if let Some(source) = &self.imported_source {
            if source.blocks_hash == self.blocks_hash() { return source.text.clone(); }
        }
        self.markdown_with_block_lines().0
    }

    /// Current block serialization and their 1-based starting lines. The editor
    /// parses this as one document so references and TOC retain global context.
    /// This does not replace the exact imported source returned by `markdown`.
    pub fn markdown_with_block_lines(&self) -> (String, Vec<usize>) {
        let parts = self.blocks
            .iter()
            .map(|b| {
                if matches!(b.kind, BlockKind::Markdown | BlockKind::Html) { return b.text.clone(); }
                if b.kind == BlockKind::Image {
                    return format!(
                        "![{}](asset:{})",
                        b.alt.replace(']', "\\]"),
                        b.asset.as_deref().unwrap_or("")
                    );
                }
                if b.kind == BlockKind::Divider {
                    return "---".into();
                }
                let mut s = String::new();
                let mut start = 0;
                while start < b.text.len() {
                    let f = b.flags_at(start);
                    let mut end = b.text.len();
                    for (off, _) in b.text[start..].char_indices().skip(1) {
                        if b.flags_at(start + off) != f {
                            end = start + off;
                            break;
                        }
                    }
                    let mut t = b.text[start..end]
                        .replace('\\', "\\\\")
                        .replace('*', "\\*")
                        .replace('_', "\\_")
                        .replace('[', "\\[")
                        .replace(']', "\\]")
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('#', "\\#")
                        .replace('>', "\\>")
                        .replace('-', "\\-")
                        .replace('+', "\\+")
                        .replace('.', "\\.")
                        .replace('`', "\\`")
                        .replace('!', "\\!");
                    if f.0 {
                        t = format!("**{t}**")
                    }
                    if f.1 {
                        t = format!("*{t}*")
                    }
                    if let Some(url) = f.2 {
                        t = format!("[{t}]({})", url.replace(')', "%29").replace('(', "%28"))
                    }
                    s.push_str(&t);
                    start = end;
                }
                let prefix = match b.kind {
                    BlockKind::Heading2 => "## ",
                    BlockKind::Heading3 => "### ",
                    BlockKind::Quote => "> ",
                    BlockKind::Bullet => "- ",
                    BlockKind::Numbered => "1. ",
                    _ => "",
                };
                let continuation = match b.kind {
                    BlockKind::Quote => "\n> ",
                    BlockKind::Bullet => "\n  ",
                    BlockKind::Numbered => "\n   ",
                    _ => "\n",
                };
                format!("{prefix}{}", s.replace('\n', continuation))
            })
            .collect::<Vec<_>>();
        let mut line = 1;
        let starts = parts.iter().map(|part| {
            let start = line;
            line += part.bytes().filter(|b| *b == b'\n').count() + 2;
            start
        }).collect();
        let mut source = parts.join("\n\n");
        if !self.reference_definitions.is_empty() {
            source.push_str("\n\n");
            source.push_str(&self.reference_definitions);
        }
        (source, starts)
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_format_survives_edit_and_markdown_roundtrip() {
        let mut d = Document::from_markdown("文章", "你好 **世界**\n\n## 山野\n\n- 松树").unwrap();
        assert_eq!(d.blocks[0].text, "你好 世界");
        assert!(d.blocks[0].inline_html().contains("<strong>世界</strong>"));
        d.blocks[0].replace_text("你好 美丽世界".into());
        assert!(d.blocks[0].inline_html().contains("<strong>世界</strong>"));
        let again = Document::from_markdown(&d.title, &d.markdown()).unwrap();
        assert_eq!(again.html(), d.html());
    }
    #[test]
    fn formatting_graphemes_not_utf8_fragments() {
        let mut b = Block::new(BlockKind::Paragraph, "你好👨‍👩‍👧‍👦hello");
        b.format(6..31, Some(true), None, None).unwrap();
        assert!(b.inline_html().contains("<strong>👨‍👩‍👧‍👦</strong>"));
        assert!(b.format(1..2, Some(true), None, None).is_err());
    }
    #[test]
    fn preserves_markup_without_activating_images_or_scripts() {
        for text in [
            "![pixel](https://bad.invalid/a.png)",
            "<script>alert(1)</script>",
            "[x](javascript:alert)",
        ] {
            let document = Document::from_markdown("A", text).unwrap();
            assert_eq!(document.markdown(), text);
            let html = document.html();
            assert!(!html.contains("<img"));
            assert!(!html.contains("<script"));
            assert!(!html.contains("javascript:"));
        }
        let d = Document::from_markdown("A", "![山谷](asset:asset_1)").unwrap();
        assert_eq!(d.asset_ids(), vec!["asset_1"]);
    }
    #[test]
    fn literal_markdown_punctuation_is_not_reinterpreted() {
        let mut doc = Document::default();
        doc.title = "Literal".into();
        doc.blocks[0].text = "# title\n- literal\n1. literal\n> literal\n`plain` & literal".into();
        let roundtrip = Document::from_markdown(&doc.title, &doc.markdown()).unwrap();
        assert_eq!(roundtrip.blocks[0].kind, BlockKind::Paragraph);
        assert_eq!(roundtrip.blocks[0].text, doc.blocks[0].text);
    }
    #[test]
    fn html_escapes_entities_once() {
        assert_eq!(escape("A & B <tag> \"x\""), "A &amp; B &lt;tag&gt; &quot;x&quot;");
    }
    #[test]
    fn quote_lines_and_literal_entities_roundtrip_without_loss() {
        let mut doc = Document::default();
        doc.title = "A".into();
        doc.blocks[0] = Block::new(BlockKind::Quote, "第一行\n第二行 &lt;literal&gt;");
        let next = Document::from_markdown(&doc.title, &doc.markdown()).unwrap();
        assert_eq!(next.blocks[0].text, doc.blocks[0].text);
        assert_eq!(next.blocks[0].kind, BlockKind::Quote);
        let nested = Document::from_markdown("A", "- outer\n  - nested").unwrap();
        assert_eq!(nested.html().matches("<ul>").count(), 2);
    }
    #[test]
    fn image_requires_asset_and_links_remain_inert_unless_https() {
        let mut doc = Document::default();
        doc.blocks.push(Block::new(BlockKind::Image, ""));
        assert!(doc.validate().is_err());
        let mut block = Block::new(BlockKind::Paragraph, "测试链接");
        assert!(block
            .format(0..6, None, None, Some(Some("javascript:alert(1)".into())))
            .is_err());
        block
            .format(0..6, None, None, Some(Some("https://example.org".into())))
            .unwrap();
        assert!(block.inline_html().contains("href=\"https://example.org\""));
        block.replace_text("测试新链接".into());
        assert!(block.inline_html().contains("测试新"));
    }
    #[test]
    fn changing_theme_preserves_document_content() {
        let mut d = Document::from_markdown("A", "**hello**").unwrap();
        let before = d.markdown();
        for t in Theme::ALL {
            d.theme = t;
            assert_eq!(d.markdown(), before);
            d.validate().unwrap();
        }
    }
}
