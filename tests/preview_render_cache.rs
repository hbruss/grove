use grove::app::App;
use grove::preview::model::{
    ImageDisplay, ImagePreview, MermaidDisplay, MermaidPreview, MermaidSource, MermaidSourceKind,
    PreviewGeneration, PreviewHeader, PreviewMetadataItem, PreviewPayload, PreviewPresentation,
};
use grove::preview::render::{
    PreviewRenderCache, line_count_from_cache, refresh_cache, visible_text_from_cache,
};
use ratatui::style::Color;

#[test]
fn app_preview_render_cache_reuses_same_generation_and_width() {
    let mut app = App::default();
    app.tabs[0].preview.generation = PreviewGeneration(1);
    app.tabs[0].preview.payload = markdown_payload();

    assert!(app.refresh_active_preview_render_cache(40));
    assert!(!app.refresh_active_preview_render_cache(40));
    assert!(app.tabs[0].preview.render_cache.is_some());
}

#[test]
fn app_preview_render_cache_invalidates_on_generation_change() {
    let mut app = App::default();
    app.tabs[0].preview.generation = PreviewGeneration(1);
    app.tabs[0].preview.payload = markdown_payload();

    assert!(app.refresh_active_preview_render_cache(40));
    app.tabs[0].preview.generation = PreviewGeneration(2);
    assert!(app.refresh_active_preview_render_cache(40));
    assert_eq!(
        app.tabs[0]
            .preview
            .render_cache
            .as_ref()
            .expect("cache should exist")
            .generation,
        PreviewGeneration(2)
    );
}

#[test]
fn preview_render_cache_reuses_same_generation_and_width() {
    let payload = markdown_payload();
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        40
    ));
    assert!(!refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        40
    ));

    let cache = cache.expect("cache should exist");
    assert_eq!(cache.generation, PreviewGeneration(1));
    assert_eq!(cache.width, 40);
    assert!(line_count_from_cache(Some(&cache)) > 0);
}

#[test]
fn preview_render_cache_invalidates_on_width_or_generation_change() {
    let payload = markdown_payload();
    let mut cache = Some(PreviewRenderCache::default());

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        40
    ));
    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        60
    ));
    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(2),
        &payload,
        PreviewPresentation::Standard,
        60
    ));

    let cache = cache.expect("cache should exist");
    assert_eq!(cache.generation, PreviewGeneration(2));
    assert_eq!(cache.width, 60);
}

#[test]
fn visible_text_from_cache_uses_cached_scroll_slice() {
    let payload = PreviewPayload {
        title: "notes.txt".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/notes.txt".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Modified".to_string(),
                value: "2026-03-19 14:05".to_string(),
            }],
        },
        lines: vec![
            "line 00".to_string(),
            "line 01".to_string(),
            "line 02".to_string(),
            "line 03".to_string(),
        ],
        markdown: None,
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 3, 2, 0, None, false)
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(visible, vec!["line 00".to_string(), "line 01".to_string()]);
}

#[test]
fn visible_text_from_cache_renders_preview_header_before_body() {
    let payload = markdown_payload();
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 3, 0, None, false)
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(visible[0].starts_with("/tmp/README.md"));
    assert!(visible[1].starts_with("Type Markdown"));
    assert!(visible[2].trim().is_empty());
}

#[test]
fn visible_text_from_cache_renders_markdown_links_inline_without_links_appendix() {
    let payload = PreviewPayload {
        title: "README.md".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/README.md".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "Markdown".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: Some("# Heading\n\nBefore [OpenAI](https://openai.com) after.\n".to_string()),
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 8, 0, None, false)
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(
        visible
            .iter()
            .any(|line| line.contains("Before OpenAI (https://openai.com) after.")),
        "inline markdown links should stay inline in the rendered body"
    );
    assert!(
        !visible.iter().any(|line| line.trim() == "Links"),
        "markdown preview should not append a detached Links section"
    );
}

#[test]
fn visible_text_from_cache_renders_fenced_code_blocks_without_raw_fences() {
    let payload = PreviewPayload {
        title: "README.md".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/README.md".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "Markdown".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: Some("```rust\nfn main() {}\n```\n".to_string()),
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 8, 0, None, false)
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(
        visible.iter().any(|line| line.contains("fn main() {}")),
        "fenced code should render as body text"
    );
    assert!(
        !visible.iter().any(|line| line.contains("```")),
        "raw fences should not leak into the rendered preview"
    );
}

#[test]
fn visible_text_from_cache_tints_preview_metadata_lockup() {
    let payload = PreviewPayload {
        title: "notes.txt".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/notes.txt".to_string()),
            metadata: vec![
                PreviewMetadataItem {
                    label: "Type".to_string(),
                    value: "Text".to_string(),
                },
                PreviewMetadataItem {
                    label: "Size".to_string(),
                    value: "5 B".to_string(),
                },
            ],
        },
        lines: vec!["hello".to_string()],
        markdown: None,
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 4, 0, None, false).lines;
    let path_bg = visible[0].spans[0].style.bg;
    let metadata_bg = visible[1].spans[0].style.bg;
    let body_bg = visible[3].spans[0].style.bg;

    assert_eq!(path_bg, metadata_bg);
    assert_ne!(path_bg, Some(Color::Reset));
    assert_ne!(path_bg, body_bg);
}

#[test]
fn markdown_render_inlines_link_targets_without_detached_links_section() {
    let payload = PreviewPayload {
        title: "README.md".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/README.md".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "Markdown".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: Some("See [OpenAI](https://openai.com) for details.\n".to_string()),
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let rendered = visible_text_from_cache(cache.as_ref(), 0, 16, 0, None, false)
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("OpenAI (https://openai.com)")),
        "expected inline markdown links to render with their URL in place"
    );
    assert!(
        !rendered.iter().any(|line| line.trim() == "Links"),
        "expected no detached links appendix once markdown is Grove-rendered"
    );
}

#[test]
fn visible_text_from_cache_highlights_preview_cursor_and_selection() {
    let payload = PreviewPayload {
        title: "notes.txt".to_string(),
        header: PreviewHeader::default(),
        lines: vec![
            "line 00".to_string(),
            "line 01".to_string(),
            "line 02".to_string(),
            "line 03".to_string(),
        ],
        markdown: None,
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 4, 2, Some((1, 3)), true).lines;
    let line_00_style = visible[0].spans[0].style;
    let line_01_style = visible[1].spans[0].style;
    let line_02_style = visible[2].spans[0].style;
    let line_03_style = visible[3].spans[0].style;

    assert_eq!(line_00_style.bg, None);
    assert_eq!(line_01_style.bg, Some(Color::Rgb(66, 76, 92)));
    assert_eq!(line_01_style.fg, Some(Color::Rgb(228, 233, 242)));
    assert_eq!(line_02_style.bg, Some(Color::Rgb(82, 95, 118)));
    assert_eq!(line_02_style.fg, Some(Color::Rgb(242, 246, 252)));
    assert_eq!(line_03_style.bg, line_01_style.bg);
    assert_eq!(line_03_style.fg, line_01_style.fg);
}

#[test]
fn visible_text_from_cache_tints_diff_added_removed_and_hunk_lines() {
    let payload = PreviewPayload {
        title: "Diff src/lib.rs".to_string(),
        header: PreviewHeader::default(),
        lines: vec![
            "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
            "--- a/src/lib.rs".to_string(),
            "+++ b/src/lib.rs".to_string(),
            "@@ -1,2 +1,2 @@".to_string(),
            "-old value".to_string(),
            "+new value".to_string(),
            " unchanged".to_string(),
        ],
        markdown: None,
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Diff,
        100
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 8, 0, None, false).lines;
    let metadata_style = visible[0].spans[0].style;
    let hunk_style = visible[3].spans[0].style;
    let removed_style = visible[4].spans[0].style;
    let added_style = visible[5].spans[0].style;
    let context_style = visible[6].spans[0].style;

    assert_eq!(metadata_style.bg, None);
    assert_eq!(hunk_style.bg, Some(Color::Rgb(42, 47, 58)));
    assert_eq!(hunk_style.fg, Some(Color::Rgb(164, 176, 194)));
    assert_eq!(removed_style.bg, Some(Color::Rgb(94, 54, 60)));
    assert_eq!(removed_style.fg, Some(Color::Rgb(250, 228, 232)));
    assert_eq!(added_style.bg, Some(Color::Rgb(46, 86, 64)));
    assert_eq!(added_style.fg, Some(Color::Rgb(227, 247, 233)));
    assert_eq!(context_style.bg, None);
}

#[test]
fn preview_render_cache_tracks_reserved_image_slot_for_mermaid_image_previews() {
    let payload = PreviewPayload {
        title: "diagram.mmd".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/diagram.mmd".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "Mermaid".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: None,
        image: None,
        mermaid: Some(MermaidPreview {
            source: MermaidSource {
                kind: MermaidSourceKind::NativeFile,
                block_index: None,
                total_blocks: 1,
                label: "Mermaid".to_string(),
                raw_source: "graph TD;A-->B;".to_string(),
            },
            display: MermaidDisplay::Image,
            status: "Mermaid diagram rendered via mmdc".to_string(),
            body_lines: Vec::new(),
        }),
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::MermaidImage,
        80
    ));

    let cache = cache.expect("cache should exist");
    let slot = cache
        .image_slot
        .as_ref()
        .expect("image presentation should reserve an overlay slot");
    assert!(
        slot.start_line > 0,
        "header rows should precede the image slot"
    );
    assert!(slot.height_lines > 0);
}

#[test]
fn preview_render_cache_tracks_reserved_image_slot_for_general_image_previews() {
    let payload = PreviewPayload {
        title: "pixel.png".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/pixel.png".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "PNG Image".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: None,
        image: Some(ImagePreview {
            display: ImageDisplay::Inline,
            status: "Image rendered inline".to_string(),
            format_label: "PNG".to_string(),
            dimensions: Some((1, 1)),
            body_lines: Vec::new(),
        }),
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::ImageInline,
        80
    ));

    let cache = cache.expect("cache should exist");
    let slot = cache
        .image_slot
        .as_ref()
        .expect("image presentation should reserve an overlay slot");
    assert!(
        slot.start_line > 0,
        "header rows should precede the general image slot"
    );
    assert!(slot.height_lines > 0);
}

#[test]
fn visible_text_from_cache_does_not_tint_plus_prefixed_plain_text_without_diff_presentation() {
    let payload = PreviewPayload {
        title: "notes.txt".to_string(),
        header: PreviewHeader::default(),
        lines: vec!["+this is plain text".to_string()],
        markdown: None,
        image: None,
        mermaid: None,
    };
    let mut cache = None;

    assert!(refresh_cache(
        &mut cache,
        PreviewGeneration(1),
        &payload,
        PreviewPresentation::Standard,
        80
    ));

    let visible = visible_text_from_cache(cache.as_ref(), 0, 2, 0, None, false).lines;
    assert_eq!(visible[0].spans[0].style.bg, None);
    assert_eq!(visible[0].spans[0].style.fg, None);
}

fn markdown_payload() -> PreviewPayload {
    PreviewPayload {
        title: "README.md".to_string(),
        header: PreviewHeader {
            path: Some("/tmp/README.md".to_string()),
            metadata: vec![PreviewMetadataItem {
                label: "Type".to_string(),
                value: "Markdown".to_string(),
            }],
        },
        lines: Vec::new(),
        markdown: Some("# Heading\n\nSome paragraph text.\n".to_string()),
        image: None,
        mermaid: None,
    }
}
