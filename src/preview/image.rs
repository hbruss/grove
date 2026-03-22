use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use image::GenericImageView;

use crate::config::PreviewConfig;
use crate::preview::model::{ImageDisplay, ImagePreview, PreviewGeneration};

pub const IMAGE_PREVIEW_HEIGHT_LINES: u16 = 18;
const IMAGE_RENDER_MAX_EDGE_PX: u32 = 1_280;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRenderKey {
    pub generation: PreviewGeneration,
    pub path: PathBuf,
    pub file_size: u64,
    pub modified_nanos: u128,
    pub graphics_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInlineImage {
    pub png_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageRenderOutcome {
    Inline(ImageInlineImage),
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRenderRequest {
    pub key: ImageRenderKey,
    pub path: PathBuf,
    pub format_label: String,
    pub max_bytes: usize,
    pub max_pixels: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRenderResponse {
    pub key: ImageRenderKey,
    pub status: String,
    pub dimensions: Option<(u32, u32)>,
    pub outcome: ImageRenderOutcome,
}

#[derive(Debug)]
pub struct ImageRenderWorker {
    shared: Arc<WorkerShared>,
    responses: mpsc::Receiver<ImageRenderResponse>,
}

impl ImageRenderWorker {
    pub fn submit(&self, request: ImageRenderRequest) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        if state.closed {
            return false;
        }
        state.pending = Some(request);
        self.shared.wake.notify_one();
        true
    }

    pub fn try_recv(&self) -> Result<ImageRenderResponse, mpsc::TryRecvError> {
        self.responses.try_recv()
    }
}

impl Drop for ImageRenderWorker {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.pending = None;
        }
        self.shared.wake.notify_one();
    }
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: Option<ImageRenderRequest>,
    closed: bool,
}

#[derive(Debug)]
struct WorkerShared {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

pub fn is_supported_image_extension(extension: Option<&str>) -> bool {
    matches!(
        extension,
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
    )
}

pub fn image_format_label(extension: Option<&str>) -> &'static str {
    match extension {
        Some("png") => "PNG",
        Some("jpg") | Some("jpeg") => "JPEG",
        Some("gif") => "GIF",
        Some("webp") => "WebP",
        _ => "Image",
    }
}

pub fn build_pending_image_preview(extension: Option<&str>) -> ImagePreview {
    ImagePreview {
        display: ImageDisplay::Pending,
        status: "Image preview pending".to_string(),
        format_label: image_format_label(extension).to_string(),
        dimensions: None,
        body_lines: vec!["Preparing inline image preview...".to_string()],
    }
}

pub fn build_render_request(
    generation: PreviewGeneration,
    abs_path: &Path,
    format_label: String,
    metadata: &fs::Metadata,
    config: &PreviewConfig,
    graphics_enabled: bool,
) -> ImageRenderRequest {
    ImageRenderRequest {
        key: ImageRenderKey {
            generation,
            path: abs_path.to_path_buf(),
            file_size: metadata.len(),
            modified_nanos: metadata_modified_nanos(metadata),
            graphics_enabled,
        },
        path: abs_path.to_path_buf(),
        format_label,
        max_bytes: config.image_preview_max_bytes,
        max_pixels: config.image_preview_max_pixels,
    }
}

pub fn start_background_image_render() -> ImageRenderWorker {
    let shared = Arc::new(WorkerShared {
        state: Mutex::new(WorkerState::default()),
        wake: Condvar::new(),
    });
    let (response_sender, responses) = mpsc::channel();
    let thread_shared = Arc::clone(&shared);

    std::thread::spawn(move || {
        loop {
            let request = {
                let Ok(mut state) = thread_shared.state.lock() else {
                    return;
                };
                while state.pending.is_none() && !state.closed {
                    state = match thread_shared.wake.wait(state) {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                }

                if state.closed {
                    return;
                }

                state.pending.take()
            };

            let Some(request) = request else {
                continue;
            };
            if response_sender.send(render_request(request)).is_err() {
                return;
            }
        }
    });

    ImageRenderWorker { shared, responses }
}

fn metadata_modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn render_request(request: ImageRenderRequest) -> ImageRenderResponse {
    let dimensions = image::image_dimensions(&request.path).ok();

    if !request.key.graphics_enabled {
        return ImageRenderResponse {
            key: request.key,
            status: "Inline image preview requires iTerm2; showing metadata summary".to_string(),
            dimensions,
            outcome: ImageRenderOutcome::Summary,
        };
    }

    if request.key.file_size > request.max_bytes as u64 {
        return ImageRenderResponse {
            key: request.key,
            status: "Image file too large for inline preview; showing metadata summary".to_string(),
            dimensions,
            outcome: ImageRenderOutcome::Summary,
        };
    }

    if dimensions
        .map(|(width, height)| u64::from(width) * u64::from(height) > request.max_pixels)
        .unwrap_or(false)
    {
        return ImageRenderResponse {
            key: request.key,
            status: "Image dimensions exceed the preview budget; showing metadata summary"
                .to_string(),
            dimensions,
            outcome: ImageRenderOutcome::Summary,
        };
    }

    let decoded = match image::ImageReader::open(&request.path) {
        Ok(reader) => match reader.with_guessed_format() {
            Ok(reader) => match reader.decode() {
                Ok(image) => image,
                Err(err) => {
                    return ImageRenderResponse {
                        key: request.key,
                        status: format!("Image decode failed: {err}; showing metadata summary"),
                        dimensions,
                        outcome: ImageRenderOutcome::Summary,
                    };
                }
            },
            Err(err) => {
                return ImageRenderResponse {
                    key: request.key,
                    status: format!(
                        "Image format detection failed: {err}; showing metadata summary"
                    ),
                    dimensions,
                    outcome: ImageRenderOutcome::Summary,
                };
            }
        },
        Err(err) => {
            return ImageRenderResponse {
                key: request.key,
                status: format!("Image preview unavailable: {err}; showing metadata summary"),
                dimensions,
                outcome: ImageRenderOutcome::Summary,
            };
        }
    };

    let inline = normalize_inline_image(decoded);
    let mut png = Cursor::new(Vec::new());
    if let Err(err) = inline.write_to(&mut png, image::ImageFormat::Png) {
        return ImageRenderResponse {
            key: request.key,
            status: format!("Image transcode failed: {err}; showing metadata summary"),
            dimensions,
            outcome: ImageRenderOutcome::Summary,
        };
    }

    ImageRenderResponse {
        key: request.key,
        status: format!("{} image rendered inline", request.format_label),
        dimensions: dimensions.or_else(|| Some((inline.width(), inline.height()))),
        outcome: ImageRenderOutcome::Inline(ImageInlineImage {
            png_bytes: png.into_inner(),
        }),
    }
}

fn normalize_inline_image(image: image::DynamicImage) -> image::DynamicImage {
    let (width, height) = image.dimensions();
    if width <= IMAGE_RENDER_MAX_EDGE_PX && height <= IMAGE_RENDER_MAX_EDGE_PX {
        return image;
    }

    image.resize(
        IMAGE_RENDER_MAX_EDGE_PX,
        IMAGE_RENDER_MAX_EDGE_PX,
        image::imageops::FilterType::Triangle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::model::PreviewGeneration;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn render_request_produces_inline_png_bytes_for_a_small_supported_image() {
        let root = make_temp_dir("grove-image-render-inline");
        let path = root.join("pixel.png");
        write_tiny_png(&path);

        let response = render_request(ImageRenderRequest {
            key: ImageRenderKey {
                generation: PreviewGeneration(1),
                path: path.clone(),
                file_size: fs::metadata(&path).expect("metadata should load").len(),
                modified_nanos: 1,
                graphics_enabled: true,
            },
            path: path.clone(),
            format_label: "PNG".to_string(),
            max_bytes: 1024 * 1024,
            max_pixels: 1024 * 1024,
        });

        match response.outcome {
            ImageRenderOutcome::Inline(image) => {
                assert!(!image.png_bytes.is_empty());
                assert_eq!(response.dimensions, Some((1, 1)));
            }
            other => panic!("expected inline image outcome, got {other:?}"),
        }

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn render_request_falls_back_to_summary_when_the_file_exceeds_the_byte_budget() {
        let root = make_temp_dir("grove-image-render-summary");
        let path = root.join("pixel.png");
        write_tiny_png(&path);

        let response = render_request(ImageRenderRequest {
            key: ImageRenderKey {
                generation: PreviewGeneration(2),
                path: path.clone(),
                file_size: fs::metadata(&path).expect("metadata should load").len(),
                modified_nanos: 2,
                graphics_enabled: true,
            },
            path: path.clone(),
            format_label: "PNG".to_string(),
            max_bytes: 8,
            max_pixels: 1024 * 1024,
        });

        assert_eq!(response.outcome, ImageRenderOutcome::Summary);
        assert!(
            response.status.to_ascii_lowercase().contains("too large"),
            "oversized image fallback should explain the byte-budget reason"
        );

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    fn write_tiny_png(path: &std::path::Path) {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("tiny png fixture should encode");
        fs::write(path, encoded.into_inner()).expect("tiny png fixture should be written");
    }
}
