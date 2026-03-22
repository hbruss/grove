use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::PreviewConfig;
use crate::preview::image::{
    build_pending_image_preview, image_format_label, is_supported_image_extension,
};
use crate::preview::mermaid::{
    build_native_mermaid_preview, detect_markdown_mermaid_preview, is_native_mermaid_extension,
};
use crate::preview::model::{PreviewHeader, PreviewMetadataItem, PreviewPayload};
use crate::tree::model::{Node, NodeKind};

pub fn load_preview(root_abs: &Path, node: &Node, config: &PreviewConfig) -> PreviewPayload {
    if matches!(node.kind, NodeKind::Directory | NodeKind::SymlinkDirectory) {
        return load_directory_preview(root_abs, node);
    }
    load_file_preview(root_abs, node, config)
}

fn load_directory_preview(root_abs: &Path, node: &Node) -> PreviewPayload {
    let abs_path = root_abs.join(&node.rel_path);
    let metadata = fs::metadata(&abs_path).ok();
    let child_count = fs::read_dir(&abs_path)
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(node.children.len());

    PreviewPayload {
        title: preview_title(node),
        header: build_preview_header(
            &abs_path,
            metadata.as_ref(),
            preview_kind_label(&node.kind, Some("folder"), None),
            vec![metadata_item("Children", child_count.to_string())],
        ),
        lines: Vec::new(),
        markdown: None,
        image: None,
        mermaid: None,
    }
}

fn load_file_preview(root_abs: &Path, node: &Node, config: &PreviewConfig) -> PreviewPayload {
    let abs_path = root_abs.join(&node.rel_path);
    let metadata = match fs::metadata(&abs_path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return PreviewPayload {
                title: preview_title(node),
                header: PreviewHeader {
                    path: Some(abs_path.display().to_string()),
                    metadata: Vec::new(),
                },
                lines: vec!["Preview unavailable".to_string(), format!("Error: {err}")],
                markdown: None,
                image: None,
                mermaid: None,
            };
        }
    };

    let extension = abs_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    if is_supported_image_extension(extension.as_deref()) {
        return PreviewPayload {
            title: preview_title(node),
            header: build_preview_header(
                &abs_path,
                Some(&metadata),
                preview_kind_label(&node.kind, Some("image"), extension.as_deref()),
                Vec::new(),
            ),
            lines: Vec::new(),
            markdown: None,
            image: Some(build_pending_image_preview(extension.as_deref())),
            mermaid: None,
        };
    }

    if metadata.len() > config.raw_text_max_bytes as u64 {
        return PreviewPayload {
            title: preview_title(node),
            header: build_preview_header(
                &abs_path,
                Some(&metadata),
                preview_kind_label(&node.kind, Some("text"), None),
                Vec::new(),
            ),
            lines: vec![
                "File too large for inline preview".to_string(),
                format!("Size: {} bytes", metadata.len()),
            ],
            markdown: None,
            image: None,
            mermaid: None,
        };
    }

    let bytes = match fs::read(&abs_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return PreviewPayload {
                title: preview_title(node),
                header: build_preview_header(
                    &abs_path,
                    Some(&metadata),
                    preview_kind_label(&node.kind, Some("file"), None),
                    Vec::new(),
                ),
                lines: vec!["Preview unavailable".to_string(), format!("Error: {err}")],
                markdown: None,
                image: None,
                mermaid: None,
            };
        }
    };

    if is_probably_binary(&bytes, config.binary_sniff_bytes) {
        return load_binary_preview(&abs_path, metadata.len(), &bytes, node);
    }

    let text = String::from_utf8_lossy(&bytes);
    match extension.as_deref() {
        ext if is_native_mermaid_extension(ext) => PreviewPayload {
            title: preview_title(node),
            header: build_preview_header(
                &abs_path,
                Some(&metadata),
                preview_kind_label(&node.kind, Some("mermaid"), extension.as_deref()),
                Vec::new(),
            ),
            lines: Vec::new(),
            markdown: None,
            image: None,
            mermaid: Some(build_native_mermaid_preview(&text)),
        },
        Some("md") | Some("markdown") => PreviewPayload {
            title: preview_title(node),
            header: build_preview_header(
                &abs_path,
                Some(&metadata),
                preview_kind_label(&node.kind, Some("markdown"), extension.as_deref()),
                Vec::new(),
            ),
            lines: Vec::new(),
            markdown: Some(text.clone().into_owned()),
            image: None,
            mermaid: detect_markdown_mermaid_preview(&text),
        },
        Some("json") => {
            let body = render_json_preview(&text).unwrap_or_else(|| plain_text_lines(&text));
            PreviewPayload {
                title: preview_title(node),
                header: build_preview_header(
                    &abs_path,
                    Some(&metadata),
                    preview_kind_label(&node.kind, Some("json"), extension.as_deref()),
                    Vec::new(),
                ),
                lines: body,
                markdown: None,
                image: None,
                mermaid: None,
            }
        }
        _ => PreviewPayload {
            title: preview_title(node),
            header: build_preview_header(
                &abs_path,
                Some(&metadata),
                preview_kind_label(&node.kind, Some("text"), extension.as_deref()),
                Vec::new(),
            ),
            lines: plain_text_lines(&text),
            markdown: None,
            image: None,
            mermaid: None,
        },
    }
}

fn is_probably_binary(bytes: &[u8], sniff_bytes: usize) -> bool {
    bytes.iter().take(sniff_bytes).any(|byte| *byte == 0)
}

fn preview_title(node: &Node) -> String {
    node.name.clone()
}

fn plain_text_lines(text: &str) -> Vec<String> {
    let mut lines = text.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("(empty file)".to_string());
    }
    lines
}

fn render_json_preview(text: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let pretty = serde_json::to_string_pretty(&value).ok()?;
    Some(plain_text_lines(&pretty))
}

fn load_binary_preview(abs_path: &Path, size: u64, bytes: &[u8], node: &Node) -> PreviewPayload {
    let mut lines = vec!["Binary preview".to_string(), String::new()];

    for (offset, chunk) in bytes.chunks(16).take(4).enumerate() {
        lines.push(format_hex_row(offset * 16, chunk));
    }

    PreviewPayload {
        title: preview_title(node),
        header: build_preview_header(
            abs_path,
            fs::metadata(abs_path).ok().as_ref(),
            preview_kind_label(&node.kind, Some("binary"), None),
            vec![metadata_item("Bytes", size.to_string())],
        ),
        lines,
        markdown: None,
        image: None,
        mermaid: None,
    }
}

fn build_preview_header(
    abs_path: &Path,
    metadata: Option<&fs::Metadata>,
    type_label: &str,
    extra_items: Vec<PreviewMetadataItem>,
) -> PreviewHeader {
    let mut items = vec![metadata_item("Type", type_label.to_string())];
    items.extend(extra_items);

    if let Some(metadata) = metadata {
        if metadata.is_file() {
            items.push(metadata_item("Size", format_bytes(metadata.len())));
        }
        if let Ok(modified) = metadata.modified()
            && let Some(value) = format_local_time(modified)
        {
            items.push(metadata_item("Modified", value));
        }
        if let Ok(created) = metadata.created()
            && let Some(value) = format_local_time(created)
        {
            items.push(metadata_item("Created", value));
        }
        if let Some(value) = format_permissions(metadata) {
            items.push(metadata_item("Perm", value));
        }
        if let Some(value) = format_owner_group(metadata) {
            items.push(metadata_item("Owner", value));
        }
    }

    PreviewHeader {
        path: Some(abs_path.display().to_string()),
        metadata: items,
    }
}

fn metadata_item(label: &str, value: String) -> PreviewMetadataItem {
    PreviewMetadataItem {
        label: label.to_string(),
        value,
    }
}

fn preview_kind_label(
    kind: &NodeKind,
    content_hint: Option<&str>,
    extension: Option<&str>,
) -> &'static str {
    match (kind, content_hint) {
        (NodeKind::Directory, _) => "Folder",
        (NodeKind::SymlinkDirectory, _) => "Link Folder",
        (NodeKind::SymlinkFile, Some("image")) => match image_format_label(extension) {
            "PNG" => "PNG Image Link",
            "JPEG" => "JPEG Image Link",
            "GIF" => "GIF Image Link",
            "WebP" => "WebP Image Link",
            _ => "Image Link",
        },
        (NodeKind::SymlinkFile, Some("markdown")) => "Link Markdown",
        (NodeKind::SymlinkFile, Some("mermaid")) => "Link Mermaid",
        (NodeKind::SymlinkFile, Some("json")) => "Link JSON",
        (NodeKind::SymlinkFile, Some("binary")) => "Link Binary",
        (NodeKind::SymlinkFile, Some("text")) => "Link Text",
        (NodeKind::SymlinkFile, _) => "Link File",
        (NodeKind::File, Some("image")) => match image_format_label(extension) {
            "PNG" => "PNG Image",
            "JPEG" => "JPEG Image",
            "GIF" => "GIF Image",
            "WebP" => "WebP Image",
            _ => "Image",
        },
        (NodeKind::File, Some("markdown")) => "Markdown",
        (NodeKind::File, Some("mermaid")) => "Mermaid",
        (NodeKind::File, Some("json")) => "JSON",
        (NodeKind::File, Some("binary")) => "Binary",
        (NodeKind::File, Some("text")) => "Text",
        (NodeKind::File, _) => "File",
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_local_time(value: SystemTime) -> Option<String> {
    let seconds = value.duration_since(UNIX_EPOCH).ok()?.as_secs();
    #[cfg(unix)]
    {
        use std::ffi::CStr;

        let timestamp = seconds as libc::time_t;
        let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
        let format = b"%Y-%m-%d %H:%M\0";
        let mut buffer = [0 as libc::c_char; 64];

        let tm_ptr = unsafe { libc::localtime_r(&timestamp, &mut tm) };
        if tm_ptr.is_null() {
            return None;
        }
        let written = unsafe {
            libc::strftime(
                buffer.as_mut_ptr(),
                buffer.len(),
                format.as_ptr().cast(),
                &tm,
            )
        };
        if written == 0 {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    }
    #[cfg(not(unix))]
    {
        Some(seconds.to_string())
    }
}

#[cfg(unix)]
fn format_permissions(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode();
    Some(format!(
        "{:03o} {}",
        mode & 0o777,
        unix_permission_bits(mode & 0o777)
    ))
}

#[cfg(not(unix))]
fn format_permissions(_metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(unix)]
fn unix_permission_bits(mode: u32) -> String {
    let flags = [
        0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
    ];
    let glyphs = ['r', 'w', 'x', 'r', 'w', 'x', 'r', 'w', 'x'];
    flags
        .into_iter()
        .zip(glyphs)
        .map(|(flag, glyph)| if mode & flag != 0 { glyph } else { '-' })
        .collect()
}

#[cfg(unix)]
fn format_owner_group(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    let user = lookup_user_name(metadata.uid()).unwrap_or_else(|| metadata.uid().to_string());
    let group = lookup_group_name(metadata.gid()).unwrap_or_else(|| metadata.gid().to_string());
    Some(format!("{user}:{group}"))
}

#[cfg(not(unix))]
fn format_owner_group(_metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(unix)]
fn lookup_user_name(uid: u32) -> Option<String> {
    let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 1024];
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(unix)]
fn lookup_group_name(gid: u32) -> Option<String> {
    let mut group = unsafe { std::mem::zeroed::<libc::group>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 1024];
    let status = unsafe {
        libc::getgrgid_r(
            gid,
            &mut group,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || group.gr_name.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(group.gr_name) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn format_hex_row(offset: usize, chunk: &[u8]) -> String {
    let hex = chunk
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let ascii = chunk
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect::<String>();
    format!("{offset:08X}  {hex:<47}  |{ascii}|")
}
