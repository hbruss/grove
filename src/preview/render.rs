use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::preview::image::IMAGE_PREVIEW_HEIGHT_LINES;
use crate::preview::mermaid::MERMAID_IMAGE_HEIGHT_LINES;
use crate::preview::model::{
    ImageDisplay, ImagePreview, MermaidDisplay, MermaidPreview, PreviewGeneration, PreviewHeader,
    PreviewMetadataItem, PreviewPayload, PreviewPresentation,
};
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewImageSlot {
    pub start_line: usize,
    pub height_lines: u16,
}

#[derive(Debug, Clone, Default)]
pub struct PreviewRenderCache {
    pub generation: PreviewGeneration,
    pub presentation: PreviewPresentation,
    pub width: u16,
    pub lines: Vec<Line<'static>>,
    pub image_slot: Option<PreviewImageSlot>,
}

#[derive(Debug, Clone)]
struct StyledChunk {
    text: String,
    style: Style,
}

#[derive(Debug, Clone)]
enum InlineBlockKind {
    Paragraph,
    Heading(HeadingLevel),
}

#[derive(Debug, Clone)]
struct InlineBlock {
    kind: InlineBlockKind,
    first_prefix: String,
    continuation_prefix: String,
}

#[derive(Debug, Clone)]
struct ListState {
    ordered: bool,
    next_number: u64,
}

#[derive(Debug, Clone)]
struct LinkState {
    url: String,
    chunks: Vec<StyledChunk>,
}

#[derive(Debug, Clone, Default)]
struct InlineStyleState {
    emphasis: bool,
    strong: bool,
    strikethrough: bool,
}

impl InlineStyleState {
    fn style(&self) -> Style {
        let mut style = markdown_body_style();
        if self.emphasis {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strong {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.strikethrough {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }
}

pub fn refresh_cache(
    cache: &mut Option<PreviewRenderCache>,
    generation: PreviewGeneration,
    payload: &PreviewPayload,
    presentation: PreviewPresentation,
    inner_width: u16,
) -> bool {
    let width = inner_width.max(1);
    if let Some(existing) = cache
        && existing.generation == generation
        && existing.presentation == presentation
        && existing.width == width
    {
        return false;
    }

    let (lines, image_slot) = lines(payload, presentation, width);
    *cache = Some(PreviewRenderCache {
        generation,
        presentation,
        width,
        lines,
        image_slot,
    });
    true
}

pub fn line_count_from_cache(cache: Option<&PreviewRenderCache>) -> usize {
    cache.map(|cache| cache.lines.len()).unwrap_or(1).max(1)
}

pub fn visible_text_from_cache(
    cache: Option<&PreviewRenderCache>,
    scroll_row: usize,
    viewport_height: usize,
    cursor_line: usize,
    selection_range: Option<(usize, usize)>,
    preview_focused: bool,
) -> Text<'static> {
    let lines = cache.map(|cache| cache.lines.as_slice()).unwrap_or(&[]);
    if lines.is_empty() {
        return Text::from("Preview unavailable");
    }

    let start = scroll_row.min(lines.len().saturating_sub(1));
    let end = if viewport_height == 0 {
        start.saturating_add(1).min(lines.len())
    } else {
        start.saturating_add(viewport_height).min(lines.len())
    };

    let mut visible_lines = lines[start..end].to_vec();
    for (offset, line) in visible_lines.iter_mut().enumerate() {
        let absolute_line = start.saturating_add(offset);
        let highlight = if let Some((selection_start, selection_end)) = selection_range {
            if absolute_line == cursor_line && preview_focused {
                Some(theme::preview_cursor_line_style(preview_focused))
            } else if absolute_line >= selection_start && absolute_line <= selection_end {
                Some(theme::preview_selected_line_style(preview_focused))
            } else {
                None
            }
        } else if preview_focused && absolute_line == cursor_line {
            Some(theme::preview_cursor_line_style(preview_focused))
        } else {
            None
        };

        if let Some(style) = highlight {
            *line = patch_line_style(line, style);
        }
    }

    Text::from(visible_lines)
}

pub fn metadata_text(header: &PreviewHeader, inner_width: u16) -> Text<'static> {
    let lines = preview_header_lines(header, inner_width.max(1));
    if lines.is_empty() {
        Text::from("Metadata unavailable")
    } else {
        Text::from(lines)
    }
}

pub fn rendered_text_range_from_cache(
    cache: Option<&PreviewRenderCache>,
    start: usize,
    end: usize,
) -> Option<String> {
    let lines = cache?.lines.as_slice();
    if lines.is_empty() {
        return None;
    }

    let start = start.min(lines.len().saturating_sub(1));
    let end = end.min(lines.len().saturating_sub(1));
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };

    Some(
        lines[start..=end]
            .iter()
            .map(|line| line.to_string().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn patch_line_style(line: &Line<'static>, style: Style) -> Line<'static> {
    let spans = line
        .spans
        .iter()
        .map(|span| Span::styled(span.content.to_string(), span.style.patch(style)))
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn lines(
    payload: &PreviewPayload,
    presentation: PreviewPresentation,
    inner_width: u16,
) -> (Vec<Line<'static>>, Option<PreviewImageSlot>) {
    let mut lines = Vec::new();

    if let Some(image) = payload.image.as_ref() {
        let (image_lines, image_slot) = render_image_preview(image, lines.len());
        lines.extend(image_lines);
        if lines.is_empty() {
            lines.push(Line::from("Preview unavailable"));
        }
        return (lines, image_slot);
    }

    if let Some(mermaid) = payload.mermaid.as_ref() {
        let (mermaid_lines, image_slot) = render_mermaid_preview(mermaid, lines.len());
        lines.extend(mermaid_lines);
        if lines.is_empty() {
            lines.push(Line::from("Preview unavailable"));
        }
        return (lines, image_slot);
    }

    if !payload.lines.is_empty() {
        lines.extend(render_plain_lines(&payload.lines, presentation));
    }

    if let Some(markdown) = payload.markdown.as_deref() {
        if !payload.lines.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(render_markdown(markdown, inner_width.max(1) as usize));
    } else if payload.lines.is_empty() {
        lines.push(Line::from("Preview unavailable"));
    }

    if lines.is_empty() {
        lines.push(Line::from("Preview unavailable"));
    }

    (lines, None)
}

fn render_image_preview(
    image: &ImagePreview,
    start_line: usize,
) -> (Vec<Line<'static>>, Option<PreviewImageSlot>) {
    let status_style = match image.display {
        ImageDisplay::Pending => markdown_quote_text_style(),
        ImageDisplay::Inline => markdown_heading_style(HeadingLevel::H6),
        ImageDisplay::Summary => markdown_quote_text_style(),
    };
    let mut lines = vec![Line::from(Span::styled(image.status.clone(), status_style))];

    if image.display == ImageDisplay::Inline {
        lines.push(Line::default());
        let image_start_line = start_line.saturating_add(lines.len());
        lines.extend(
            std::iter::repeat_with(Line::default).take(IMAGE_PREVIEW_HEIGHT_LINES as usize),
        );
        return (
            lines,
            Some(PreviewImageSlot {
                start_line: image_start_line,
                height_lines: IMAGE_PREVIEW_HEIGHT_LINES,
            }),
        );
    }

    if !image.body_lines.is_empty() {
        lines.push(Line::default());
        lines.extend(image.body_lines.iter().cloned().map(Line::from));
    }

    (lines, None)
}

fn render_plain_lines(lines: &[String], presentation: PreviewPresentation) -> Vec<Line<'static>> {
    match presentation {
        PreviewPresentation::Standard
        | PreviewPresentation::ImagePending
        | PreviewPresentation::ImageInline
        | PreviewPresentation::ImageSummary
        | PreviewPresentation::MermaidPending
        | PreviewPresentation::MermaidAscii
        | PreviewPresentation::MermaidImage
        | PreviewPresentation::MermaidRawSource => lines.iter().cloned().map(Line::from).collect(),
        PreviewPresentation::Diff => render_diff_lines(lines),
    }
}

fn render_mermaid_preview(
    mermaid: &MermaidPreview,
    start_line: usize,
) -> (Vec<Line<'static>>, Option<PreviewImageSlot>) {
    let status_style = match mermaid.display {
        MermaidDisplay::Pending => markdown_quote_text_style(),
        MermaidDisplay::Ascii => markdown_table_style(),
        MermaidDisplay::Image => markdown_heading_style(HeadingLevel::H6),
        MermaidDisplay::RawSource => markdown_quote_text_style(),
    };
    let mut lines = vec![Line::from(Span::styled(
        mermaid.status.clone(),
        status_style,
    ))];

    if mermaid.display == MermaidDisplay::Image {
        lines.push(Line::default());
        let image_start_line = start_line.saturating_add(lines.len());
        lines.extend(
            std::iter::repeat_with(Line::default).take(MERMAID_IMAGE_HEIGHT_LINES as usize),
        );
        return (
            lines,
            Some(PreviewImageSlot {
                start_line: image_start_line,
                height_lines: MERMAID_IMAGE_HEIGHT_LINES,
            }),
        );
    }

    if !mermaid.body_lines.is_empty() {
        lines.push(Line::default());
        lines.extend(mermaid.body_lines.iter().cloned().map(Line::from));
    }

    (lines, None)
}

fn render_diff_lines(lines: &[String]) -> Vec<Line<'static>> {
    let mut rendered = Vec::with_capacity(lines.len());
    let mut in_hunk = false;

    for line in lines {
        let style = if line.starts_with("@@") {
            in_hunk = true;
            Some(theme::preview_diff_hunk_header_style())
        } else if is_diff_metadata_line(line) {
            in_hunk = false;
            None
        } else if in_hunk && line.starts_with('+') {
            Some(theme::preview_diff_added_line_style())
        } else if in_hunk && line.starts_with('-') {
            Some(theme::preview_diff_removed_line_style())
        } else {
            None
        };

        rendered.push(match style {
            Some(style) => Line::from(Span::styled(line.clone(), style)),
            None => Line::from(line.clone()),
        });
    }

    rendered
}

fn is_diff_metadata_line(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("Binary files ")
}

fn render_markdown(markdown: &str, width: usize) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::new(width.max(1));
    renderer.render(markdown);
    renderer.finish();
    renderer.lines
}

struct MarkdownRenderer {
    width: usize,
    lines: Vec<Line<'static>>,
    current_block: Option<InlineBlock>,
    current_chunks: Vec<StyledChunk>,
    inline_style: InlineStyleState,
    current_link: Option<LinkState>,
    quote_depth: usize,
    list_stack: Vec<ListState>,
    current_item_prefix: Option<String>,
    current_table_row: Vec<String>,
    current_table_cell_chunks: Vec<StyledChunk>,
    table_header_rows: usize,
    in_table: bool,
    code_block: Option<(String, String, String)>,
}

impl MarkdownRenderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            current_block: None,
            current_chunks: Vec::new(),
            inline_style: InlineStyleState::default(),
            current_link: None,
            quote_depth: 0,
            list_stack: Vec::new(),
            current_item_prefix: None,
            current_table_row: Vec::new(),
            current_table_cell_chunks: Vec::new(),
            table_header_rows: 0,
            in_table: false,
            code_block: None,
        }
    }

    fn render(&mut self, markdown: &str) {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_STRIKETHROUGH);

        for event in Parser::new_ext(markdown, options) {
            match event {
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                Event::Text(text) => self.push_text(text.as_ref(), self.inline_style.style()),
                Event::Code(text) => self.push_text(text.as_ref(), markdown_inline_code_style()),
                Event::SoftBreak => self.push_text(" ", self.inline_style.style()),
                Event::HardBreak => self.push_text("\n", self.inline_style.style()),
                Event::Rule => {
                    self.finish_pending_blocks();
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(self.width.min(48)),
                        markdown_rule_style(),
                    )));
                    self.push_blank_line();
                }
                Event::TaskListMarker(checked) => self.push_text(
                    if checked { "[x] " } else { "[ ] " },
                    markdown_list_prefix_style(),
                ),
                Event::Html(html) | Event::InlineHtml(html) => {
                    self.push_text(html.as_ref(), markdown_quote_text_style())
                }
                Event::FootnoteReference(label) => {
                    self.push_text(format!("[{label}]").as_str(), markdown_link_url_style())
                }
                Event::InlineMath(text) | Event::DisplayMath(text) => {
                    self.push_text(text.as_ref(), markdown_inline_code_style())
                }
            }
        }
    }

    fn finish(&mut self) {
        self.finish_pending_blocks();
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.start_inline_block(InlineBlockKind::Paragraph),
            Tag::Heading { level, .. } => self.start_inline_block(InlineBlockKind::Heading(level)),
            Tag::BlockQuote(_) => {
                self.finish_pending_blocks();
                self.quote_depth += 1;
            }
            Tag::List(start) => {
                self.finish_pending_blocks();
                self.list_stack.push(ListState {
                    ordered: start.is_some(),
                    next_number: start.unwrap_or(1),
                });
            }
            Tag::Item => {
                self.finish_pending_blocks();
                let Some(list) = self.list_stack.last_mut() else {
                    return;
                };
                self.current_item_prefix = Some(if list.ordered {
                    let prefix = format!("{}. ", list.next_number);
                    list.next_number += 1;
                    prefix
                } else {
                    "• ".to_string()
                });
            }
            Tag::CodeBlock(_) => {
                self.finish_pending_blocks();
                let (first_prefix, continuation_prefix) = self.current_prefixes();
                self.code_block = Some((String::new(), first_prefix, continuation_prefix));
            }
            Tag::Emphasis => self.inline_style.emphasis = true,
            Tag::Strong => self.inline_style.strong = true,
            Tag::Strikethrough => self.inline_style.strikethrough = true,
            Tag::Link { dest_url, .. } => {
                self.current_link = Some(LinkState {
                    url: dest_url.to_string(),
                    chunks: Vec::new(),
                });
            }
            Tag::Table(_) => {
                self.finish_pending_blocks();
                self.in_table = true;
                self.current_table_row.clear();
                self.current_table_cell_chunks.clear();
                self.table_header_rows = 0;
            }
            Tag::TableHead => self.table_header_rows = 1,
            Tag::TableRow => self.current_table_row.clear(),
            Tag::TableCell => self.current_table_cell_chunks.clear(),
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let add_blank = self.current_item_prefix.is_none() && self.quote_depth == 0;
                self.finish_inline_block(add_blank);
            }
            TagEnd::Heading(_) => self.finish_inline_block(true),
            TagEnd::BlockQuote(_) => {
                self.finish_pending_blocks();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                if !self.in_table {
                    self.push_blank_line();
                }
            }
            TagEnd::List(_) => {
                self.finish_pending_blocks();
                self.list_stack.pop();
                self.push_blank_line();
            }
            TagEnd::Item => {
                self.finish_pending_blocks();
                self.current_item_prefix = None;
            }
            TagEnd::CodeBlock => {
                if let Some((text, first_prefix, continuation_prefix)) = self.code_block.take() {
                    for line in text.lines() {
                        let chunks = vec![StyledChunk {
                            text: line.to_string(),
                            style: markdown_code_block_style(),
                        }];
                        self.lines.extend(render_wrapped_chunks(
                            &chunks,
                            &first_prefix,
                            &continuation_prefix,
                            markdown_code_block_style(),
                            self.width,
                        ));
                    }
                    if text.is_empty() {
                        self.lines.extend(render_wrapped_chunks(
                            &[StyledChunk {
                                text: String::new(),
                                style: markdown_code_block_style(),
                            }],
                            &first_prefix,
                            &continuation_prefix,
                            markdown_code_block_style(),
                            self.width,
                        ));
                    }
                    self.push_blank_line();
                }
            }
            TagEnd::Emphasis => self.inline_style.emphasis = false,
            TagEnd::Strong => self.inline_style.strong = false,
            TagEnd::Strikethrough => self.inline_style.strikethrough = false,
            TagEnd::Link => {
                if let Some(link) = self.current_link.take() {
                    for chunk in link.chunks {
                        self.current_chunks.push(StyledChunk {
                            text: chunk.text,
                            style: markdown_link_label_style().patch(chunk.style),
                        });
                    }
                    self.current_chunks.push(StyledChunk {
                        text: format!(" ({})", link.url),
                        style: markdown_link_url_style(),
                    });
                }
            }
            TagEnd::TableCell => {
                let text = self
                    .current_table_cell_chunks
                    .iter()
                    .map(|chunk| chunk.text.as_str())
                    .collect::<String>()
                    .trim()
                    .to_string();
                self.current_table_row.push(text);
                self.current_table_cell_chunks.clear();
            }
            TagEnd::TableRow => {
                let row = std::mem::take(&mut self.current_table_row);
                if !row.is_empty() {
                    let joined = row.join(" | ");
                    self.lines.extend(render_wrapped_chunks(
                        &[StyledChunk {
                            text: joined,
                            style: markdown_table_style(),
                        }],
                        "",
                        "",
                        markdown_table_style(),
                        self.width,
                    ));
                    if self.table_header_rows == 1 {
                        let separator = row
                            .iter()
                            .map(|cell| "-".repeat(cell.chars().count().max(3)))
                            .collect::<Vec<_>>()
                            .join(" | ");
                        self.lines.extend(render_wrapped_chunks(
                            &[StyledChunk {
                                text: separator,
                                style: markdown_table_separator_style(),
                            }],
                            "",
                            "",
                            markdown_table_separator_style(),
                            self.width,
                        ));
                        self.table_header_rows = 0;
                    }
                }
            }
            TagEnd::Table => {
                self.in_table = false;
                self.push_blank_line();
            }
            _ => {}
        }
    }

    fn finish_pending_blocks(&mut self) {
        if self.current_block.is_some() {
            self.finish_inline_block(false);
        }
        if let Some(link) = self.current_link.take() {
            for chunk in link.chunks {
                self.current_chunks.push(chunk);
            }
            self.current_chunks.push(StyledChunk {
                text: format!(" ({})", link.url),
                style: markdown_link_url_style(),
            });
        }
    }

    fn start_inline_block(&mut self, kind: InlineBlockKind) {
        self.finish_pending_blocks();
        let (first_prefix, continuation_prefix) = self.current_prefixes();
        self.current_block = Some(InlineBlock {
            kind,
            first_prefix,
            continuation_prefix,
        });
    }

    fn push_text(&mut self, text: &str, style: Style) {
        if let Some((buffer, ..)) = self.code_block.as_mut() {
            buffer.push_str(text);
            return;
        }
        if self.in_table && !self.current_table_cell_chunks.is_empty() {
            self.current_table_cell_chunks.push(StyledChunk {
                text: text.to_string(),
                style,
            });
            return;
        }
        if self.current_block.is_none() {
            self.start_inline_block(InlineBlockKind::Paragraph);
        }
        if let Some(link) = self.current_link.as_mut() {
            link.chunks.push(StyledChunk {
                text: text.to_string(),
                style,
            });
        } else {
            self.current_chunks.push(StyledChunk {
                text: text.to_string(),
                style,
            });
        }
    }

    fn finish_inline_block(&mut self, add_blank_line: bool) {
        let Some(block) = self.current_block.take() else {
            return;
        };
        let base_style = match block.kind {
            InlineBlockKind::Paragraph => {
                if self.quote_depth > 0 {
                    markdown_quote_text_style()
                } else {
                    markdown_body_style()
                }
            }
            InlineBlockKind::Heading(level) => markdown_heading_style(level),
        };
        let chunks = std::mem::take(&mut self.current_chunks);
        if !chunks.is_empty() {
            self.lines.extend(render_wrapped_chunks(
                &chunks,
                &block.first_prefix,
                &block.continuation_prefix,
                base_style,
                self.width,
            ));
        }
        if add_blank_line {
            self.push_blank_line();
        }
    }

    fn current_prefixes(&self) -> (String, String) {
        let quote_prefix = "│ ".repeat(self.quote_depth);
        if let Some(item_prefix) = &self.current_item_prefix {
            let continuation = " ".repeat(item_prefix.chars().count());
            (
                format!("{quote_prefix}{item_prefix}"),
                format!("{quote_prefix}{continuation}"),
            )
        } else {
            (quote_prefix.clone(), quote_prefix)
        }
    }

    fn push_blank_line(&mut self) {
        if self
            .lines
            .last()
            .is_some_and(|line| line.to_string().trim().is_empty())
        {
            return;
        }
        self.lines.push(Line::default());
    }
}

fn preview_header_lines(header: &PreviewHeader, inner_width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(path) = header.path.as_deref() {
        lines.push(header_band_line(
            vec![Span::styled(
                path.to_string(),
                header_band_style(theme::preview_path_style()),
            )],
            inner_width,
        ));
    }
    if !header.metadata.is_empty() {
        lines.extend(preview_metadata_lines(&header.metadata, inner_width));
    }
    lines
}

fn preview_metadata_lines(items: &[PreviewMetadataItem], inner_width: u16) -> Vec<Line<'static>> {
    let max_width = inner_width.max(24) as usize;
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut line_width = 0usize;

    for item in items {
        let item_width = item.label.len() + item.value.len() + 1;
        let separator_width = if spans.is_empty() { 0 } else { 3 };
        if !spans.is_empty() && line_width + separator_width + item_width > max_width {
            lines.push(header_band_line(std::mem::take(&mut spans), inner_width));
            line_width = 0;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" | ", header_band_style(Style::default())));
            line_width += 3;
        }
        spans.push(Span::styled(
            format!("{} ", item.label),
            header_band_style(theme::preview_metadata_label_style()),
        ));
        spans.push(Span::styled(
            item.value.clone(),
            header_band_style(theme::preview_metadata_value_style()),
        ));
        line_width += item_width;
    }

    if !spans.is_empty() {
        lines.push(header_band_line(spans, inner_width));
    }

    lines
}

fn header_band_style(style: Style) -> Style {
    theme::preview_header_band_style().patch(style)
}

fn header_band_line(mut spans: Vec<Span<'static>>, inner_width: u16) -> Line<'static> {
    let fill_width = inner_width.max(1) as usize;
    let line_width = spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    if line_width < fill_width {
        spans.push(Span::styled(
            " ".repeat(fill_width - line_width),
            theme::preview_header_band_style(),
        ));
    }
    Line::from(spans)
}

fn render_wrapped_chunks(
    chunks: &[StyledChunk],
    first_prefix: &str,
    continuation_prefix: &str,
    base_style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut builder = WrappedLineBuilder::new(first_prefix, markdown_prefix_style());

    for chunk in chunks {
        for token in tokenize(&chunk.text) {
            if token == "\n" {
                lines.push(builder.finish());
                builder = WrappedLineBuilder::new(continuation_prefix, markdown_prefix_style());
                continue;
            }

            let style = base_style.patch(chunk.style);
            if token.chars().all(char::is_whitespace) && !builder.has_body() {
                continue;
            }

            let token_width = display_width(&token);
            if builder.has_body() && builder.width + token_width > width.max(1) {
                lines.push(builder.finish());
                builder = WrappedLineBuilder::new(continuation_prefix, markdown_prefix_style());
                if token.chars().all(char::is_whitespace) {
                    continue;
                }
            }

            if builder.width + token_width <= width.max(1) {
                builder.push(&token, style);
                continue;
            }

            for ch in token.chars() {
                if builder.has_body() && builder.width + 1 > width.max(1) {
                    lines.push(builder.finish());
                    builder = WrappedLineBuilder::new(continuation_prefix, markdown_prefix_style());
                }
                if ch.is_whitespace() && !builder.has_body() {
                    continue;
                }
                builder.push_char(ch, style);
            }
        }
    }

    if builder.has_anything() || lines.is_empty() {
        lines.push(builder.finish());
    }

    lines
}

#[derive(Default)]
struct WrappedLineBuilder {
    spans: Vec<Span<'static>>,
    width: usize,
    prefix_width: usize,
}

impl WrappedLineBuilder {
    fn new(prefix: &str, prefix_style: Style) -> Self {
        let prefix_width = display_width(prefix);
        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix.to_string(), prefix_style));
        }
        Self {
            spans,
            width: prefix_width,
            prefix_width,
        }
    }

    fn has_body(&self) -> bool {
        self.width > self.prefix_width
    }

    fn has_anything(&self) -> bool {
        self.width > 0 || !self.spans.is_empty()
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.width += display_width(text);
        if let Some(last) = self.spans.last_mut()
            && last.style == style
        {
            last.content.to_mut().push_str(text);
        } else {
            self.spans.push(Span::styled(text.to_string(), style));
        }
    }

    fn push_char(&mut self, ch: char, style: Style) {
        let mut buffer = [0u8; 4];
        self.push(ch.encode_utf8(&mut buffer), style);
    }

    fn finish(self) -> Line<'static> {
        Line::from(self.spans)
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_whitespace = None;

    for ch in text.chars() {
        if ch == '\n' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current_is_whitespace = None;
            tokens.push("\n".to_string());
            continue;
        }

        let is_whitespace = ch.is_whitespace();
        if current_is_whitespace != Some(is_whitespace) && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        current_is_whitespace = Some(is_whitespace);
        current.push(ch);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn markdown_body_style() -> Style {
    theme::panel_style()
}

fn markdown_heading_style(level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::default()
            .fg(Color::Rgb(106, 176, 255))
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H2 => Style::default()
            .fg(Color::Rgb(218, 196, 121))
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::default()
            .fg(Color::Rgb(232, 236, 242))
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Rgb(198, 208, 223))
            .add_modifier(Modifier::BOLD),
    }
}

fn markdown_inline_code_style() -> Style {
    Style::default()
        .fg(Color::Rgb(236, 239, 244))
        .bg(Color::Rgb(42, 48, 58))
}

fn markdown_code_block_style() -> Style {
    Style::default()
        .fg(Color::Rgb(222, 228, 236))
        .bg(Color::Rgb(29, 34, 40))
}

fn markdown_link_label_style() -> Style {
    Style::default()
        .fg(Color::Rgb(130, 193, 219))
        .add_modifier(Modifier::UNDERLINED)
}

fn markdown_link_url_style() -> Style {
    Style::default().fg(Color::Rgb(144, 202, 119))
}

fn markdown_quote_text_style() -> Style {
    Style::default().fg(Color::Rgb(186, 195, 210))
}

fn markdown_prefix_style() -> Style {
    Style::default().fg(Color::Rgb(108, 118, 132))
}

fn markdown_list_prefix_style() -> Style {
    Style::default()
        .fg(Color::Rgb(214, 198, 145))
        .add_modifier(Modifier::BOLD)
}

fn markdown_rule_style() -> Style {
    Style::default().fg(Color::Rgb(82, 94, 108))
}

fn markdown_table_style() -> Style {
    Style::default().fg(Color::Rgb(204, 214, 226))
}

fn markdown_table_separator_style() -> Style {
    Style::default().fg(Color::Rgb(108, 118, 132))
}
