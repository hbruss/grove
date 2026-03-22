use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::config::PreviewConfig;
use crate::preview::model::{
    MermaidDisplay, MermaidPreview, MermaidSource, MermaidSourceKind, PreviewGeneration,
};

pub const MERMAID_IMAGE_HEIGHT_LINES: u16 = 16;
const MERMAID_RENDER_WIDTH_PX: u16 = 640;
const MERMAID_RENDER_HEIGHT_PX: u16 = 480;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MermaidRenderDiscovery {
    pub rich_command: Option<PathBuf>,
    pub ascii_helper_command: Option<PathBuf>,
}

impl MermaidRenderDiscovery {
    pub fn preferred_fallback(&self) -> MermaidDisplay {
        if self.rich_command.is_some() {
            MermaidDisplay::Pending
        } else if self.ascii_helper_command.is_some() {
            MermaidDisplay::Ascii
        } else {
            MermaidDisplay::RawSource
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MermaidRenderKey {
    pub generation: PreviewGeneration,
    pub content_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidInlineImage {
    pub png_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MermaidRenderOutcome {
    Image(MermaidInlineImage),
    Ascii(Vec<String>),
    RawSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidRenderRequest {
    pub key: MermaidRenderKey,
    pub source: MermaidSource,
    pub discovery: MermaidRenderDiscovery,
    pub graphics_enabled: bool,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidRenderResponse {
    pub key: MermaidRenderKey,
    pub status: String,
    pub outcome: MermaidRenderOutcome,
}

#[derive(Debug)]
pub struct MermaidRenderWorker {
    shared: Arc<WorkerShared>,
    responses: mpsc::Receiver<MermaidRenderResponse>,
}

impl MermaidRenderWorker {
    pub fn submit(&self, request: MermaidRenderRequest) -> bool {
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

    pub fn try_recv(&self) -> Result<MermaidRenderResponse, mpsc::TryRecvError> {
        self.responses.try_recv()
    }
}

impl Drop for MermaidRenderWorker {
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
    pending: Option<MermaidRenderRequest>,
    closed: bool,
}

#[derive(Debug)]
struct WorkerShared {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

pub fn start_background_mermaid_render() -> MermaidRenderWorker {
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
                    break;
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
            let response = render_request(request);
            if response_sender.send(response).is_err() {
                return;
            }
        }
    });

    MermaidRenderWorker { shared, responses }
}

pub fn is_native_mermaid_extension(extension: Option<&str>) -> bool {
    matches!(extension, Some("mmd") | Some("mermaid"))
}

pub fn discover_renderers(config: &PreviewConfig) -> MermaidRenderDiscovery {
    let current_exe = std::env::current_exe().ok();
    discover_renderers_with_path(
        config,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        std::env::var_os("PATH"),
        current_exe.as_deref(),
    )
}

pub fn build_render_request(
    generation: PreviewGeneration,
    preview: &MermaidPreview,
    config: &PreviewConfig,
    discovery: MermaidRenderDiscovery,
    graphics_enabled: bool,
) -> MermaidRenderRequest {
    MermaidRenderRequest {
        key: MermaidRenderKey {
            generation,
            content_hash: render_content_hash(&preview.source, &discovery, graphics_enabled),
        },
        source: preview.source.clone(),
        discovery,
        graphics_enabled,
        timeout_ms: config.mermaid_render_timeout_ms,
    }
}

pub fn inline_images_supported() -> bool {
    std::env::var("TERM_PROGRAM")
        .map(|value| value == "iTerm.app")
        .unwrap_or(false)
}

fn discover_renderers_with_path(
    config: &PreviewConfig,
    helper_root: &Path,
    path_env: Option<std::ffi::OsString>,
    current_exe: Option<&Path>,
) -> MermaidRenderDiscovery {
    let rich_command = config
        .mermaid_command
        .as_deref()
        .and_then(|command| resolve_command(command, path_env.as_deref()))
        .or_else(|| resolve_command("mmdc", path_env.as_deref()));

    let ascii_helper_command = if rich_command.is_some() {
        discover_ascii_helper(helper_root, current_exe)
    } else {
        None
    };

    MermaidRenderDiscovery {
        rich_command,
        ascii_helper_command,
    }
}

pub fn build_native_mermaid_preview(source: &str) -> MermaidPreview {
    MermaidPreview {
        source: MermaidSource {
            kind: MermaidSourceKind::NativeFile,
            block_index: None,
            total_blocks: 1,
            label: "Mermaid".to_string(),
            raw_source: source.to_string(),
        },
        display: MermaidDisplay::Pending,
        status: "Mermaid render pending".to_string(),
        body_lines: raw_source_lines(source),
    }
}

pub fn detect_markdown_mermaid_preview(markdown: &str) -> Option<MermaidPreview> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut total_blocks = 0usize;
    let mut first_block = None;
    let mut current_block_index = None;
    let mut collecting = false;
    let mut current_source = String::new();

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if is_mermaid_info_string(info.as_ref()) =>
            {
                current_block_index = Some(total_blocks);
                total_blocks = total_blocks.saturating_add(1);
                collecting = true;
                current_source.clear();
            }
            Event::Text(text) if collecting => current_source.push_str(text.as_ref()),
            Event::SoftBreak | Event::HardBreak if collecting => current_source.push('\n'),
            Event::End(TagEnd::CodeBlock) if collecting => {
                if first_block.is_none()
                    && let Some(block_index) = current_block_index
                {
                    first_block = Some((block_index, current_source.clone()));
                }
                collecting = false;
                current_block_index = None;
                current_source.clear();
            }
            _ => {}
        }
    }

    let (block_index, raw_source) = first_block?;
    Some(MermaidPreview {
        source: MermaidSource {
            kind: MermaidSourceKind::MarkdownFence,
            block_index: Some(block_index),
            total_blocks,
            label: format!("Mermaid block {} of {}", block_index + 1, total_blocks),
            raw_source: raw_source.clone(),
        },
        display: MermaidDisplay::Pending,
        status: "Mermaid render pending".to_string(),
        body_lines: raw_source_lines(&raw_source),
    })
}

fn raw_source_lines(source: &str) -> Vec<String> {
    let mut lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("(empty Mermaid source)".to_string());
    }
    lines
}

fn is_mermaid_info_string(info: &str) -> bool {
    info.split_ascii_whitespace()
        .next()
        .map(|segment| segment.eq_ignore_ascii_case("mermaid"))
        .unwrap_or(false)
}

fn discover_ascii_helper(helper_root: &Path, current_exe: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = vec![helper_root.join("tools").join("mermaid")];

    if let Some(prefix) = current_exe
        .and_then(|exe| exe.parent())
        .and_then(|bin_dir| bin_dir.parent())
    {
        candidates.push(prefix.join("share").join("grove").join("mermaid"));
    }

    for helper_dir in candidates {
        let helper_path = helper_dir.join("render_ascii.mjs");
        let package_manifest = helper_dir
            .join("node_modules")
            .join("beautiful-mermaid")
            .join("package.json");
        if helper_path.is_file() && package_manifest.is_file() {
            return Some(helper_path);
        }
    }

    None
}

fn render_request(request: MermaidRenderRequest) -> MermaidRenderResponse {
    if request.graphics_enabled
        && let Some(command) = request.discovery.rich_command.as_deref()
    {
        match render_png_with_mmdc(command, &request.source.raw_source, request.timeout_ms) {
            Ok(png_bytes) => {
                return MermaidRenderResponse {
                    key: request.key,
                    status: "Mermaid diagram rendered via mmdc".to_string(),
                    outcome: MermaidRenderOutcome::Image(MermaidInlineImage { png_bytes }),
                };
            }
            Err(err) => {
                if let Some(helper) = request.discovery.ascii_helper_command.as_deref()
                    && let Ok(lines) = render_ascii_with_helper(
                        helper,
                        &request.source.raw_source,
                        request.timeout_ms,
                    )
                {
                    return MermaidRenderResponse {
                        key: request.key,
                        status: format!(
                            "Mermaid image render failed ({err}); rendered as text instead"
                        ),
                        outcome: MermaidRenderOutcome::Ascii(lines),
                    };
                }
                return MermaidRenderResponse {
                    key: request.key,
                    status: format!("Mermaid render failed: {err}; showing raw source"),
                    outcome: MermaidRenderOutcome::RawSource,
                };
            }
        }
    }

    if let Some(helper) = request.discovery.ascii_helper_command.as_deref() {
        match render_ascii_with_helper(helper, &request.source.raw_source, request.timeout_ms) {
            Ok(lines) => {
                return MermaidRenderResponse {
                    key: request.key,
                    status: "Mermaid diagram rendered as text via beautiful-mermaid".to_string(),
                    outcome: MermaidRenderOutcome::Ascii(lines),
                };
            }
            Err(err) => {
                return MermaidRenderResponse {
                    key: request.key,
                    status: format!("Mermaid text render failed: {err}; showing raw source"),
                    outcome: MermaidRenderOutcome::RawSource,
                };
            }
        }
    }

    let status = if request.graphics_enabled {
        "Mermaid renderer unavailable; showing raw source".to_string()
    } else {
        "Mermaid inline images are unavailable in this terminal; showing raw source".to_string()
    };
    MermaidRenderResponse {
        key: request.key,
        status,
        outcome: MermaidRenderOutcome::RawSource,
    }
}

fn render_png_with_mmdc(command: &Path, source: &str, timeout_ms: u64) -> Result<Vec<u8>, String> {
    let output_path = mermaid_png_output_path();
    let args = vec![
        "-i".to_string(),
        "-".to_string(),
        "-o".to_string(),
        output_path.display().to_string(),
        "-e".to_string(),
        "png".to_string(),
        "-w".to_string(),
        MERMAID_RENDER_WIDTH_PX.to_string(),
        "-H".to_string(),
        MERMAID_RENDER_HEIGHT_PX.to_string(),
        "-b".to_string(),
        "transparent".to_string(),
        "-q".to_string(),
    ];
    let command_result = run_command_with_input(command, &args, source, timeout_ms);
    if let Err(err) = command_result {
        let _ = fs::remove_file(&output_path);
        return Err(err);
    }

    let png_bytes = match fs::read(&output_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            let _ = fs::remove_file(&output_path);
            return Err(format!("failed to read Mermaid render output: {err}"));
        }
    };
    let _ = fs::remove_file(&output_path);
    if png_bytes.is_empty() {
        return Err("mmdc produced no image bytes".to_string());
    }
    Ok(png_bytes)
}

fn render_ascii_with_helper(
    helper: &Path,
    source: &str,
    timeout_ms: u64,
) -> Result<Vec<String>, String> {
    let args = vec![helper.to_string_lossy().into_owned(), "-".to_string()];
    let output = run_command_with_input(Path::new("node"), &args, source, timeout_ms)?;
    let text = String::from_utf8(output.stdout)
        .map_err(|err| format!("helper output was not valid UTF-8: {err}"))?;
    let mut lines = text.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("(empty Mermaid text render)".to_string());
    }
    Ok(lines)
}

fn run_command_with_input(
    command: &Path,
    args: &[String],
    input: &str,
    timeout_ms: u64,
) -> Result<std::process::Output, String> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn {}: {err}", command.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|err| format!("failed to write renderer stdin: {err}"))?;
    }

    let timeout = Duration::from_millis(timeout_ms.max(1));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|err| format!("failed to collect renderer output: {err}"))?;
                if output.status.success() {
                    return Ok(output);
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = if stderr.is_empty() {
                    format!("renderer exited with status {}", output.status)
                } else {
                    stderr
                };
                return Err(message);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "renderer timed out after {} ms",
                    timeout.as_millis()
                ));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to poll renderer process: {err}"));
            }
        }
    }
}

fn render_content_hash(
    source: &MermaidSource,
    discovery: &MermaidRenderDiscovery,
    graphics_enabled: bool,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.kind.hash(&mut hasher);
    source.block_index.hash(&mut hasher);
    source.total_blocks.hash(&mut hasher);
    source.label.hash(&mut hasher);
    source.raw_source.hash(&mut hasher);
    discovery.rich_command.hash(&mut hasher);
    discovery.ascii_helper_command.hash(&mut hasher);
    graphics_enabled.hash(&mut hasher);
    hasher.finish()
}

fn mermaid_png_output_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let unique = format!("grove-mermaid-{}-{}.png", std::process::id(), nanos);
    std::env::temp_dir().join(unique)
}

fn resolve_command(command: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }

    find_on_path(command, path_env)
}

fn find_on_path(command: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path_env = path_env?;
    for directory in std::env::split_paths(path_env) {
        let candidate = directory.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PreviewConfig;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn discovery_prefers_configured_mermaid_command_over_path_lookup() {
        let root = make_temp_dir("grove-mermaid-discovery-config");
        let custom = root.join("custom-mmdc");
        let path_mmdc = root.join("bin").join("mmdc");
        write_fake_executable(&custom);
        write_fake_executable(&path_mmdc);

        let config = PreviewConfig {
            mermaid_command: Some(custom.display().to_string()),
            ..PreviewConfig::default()
        };

        let discovery = discover_renderers_with_path(
            &config,
            root.as_path(),
            Some(path_env(&[path_mmdc
                .parent()
                .expect("bin dir should exist")])),
            None,
        );

        assert_eq!(discovery.rich_command.as_deref(), Some(custom.as_path()));
        assert_eq!(discovery.preferred_fallback(), MermaidDisplay::Pending);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn discovery_finds_mmdc_on_path_without_config_override() {
        let root = make_temp_dir("grove-mermaid-discovery-path");
        let path_mmdc = root.join("bin").join("mmdc");
        write_fake_executable(&path_mmdc);

        let discovery = discover_renderers_with_path(
            &PreviewConfig::default(),
            root.as_path(),
            Some(path_env(&[path_mmdc
                .parent()
                .expect("bin dir should exist")])),
            None,
        );

        assert_eq!(discovery.rich_command.as_deref(), Some(path_mmdc.as_path()));
        assert_eq!(discovery.ascii_helper_command, None);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn discovery_only_enables_beautiful_helper_when_mmdc_is_available() {
        let root = make_temp_dir("grove-mermaid-discovery-beautiful");
        let helper_root = root.join("tools").join("mermaid");
        fs::create_dir_all(helper_root.join("node_modules").join("beautiful-mermaid"))
            .expect("helper package dir should exist");
        fs::write(
            helper_root
                .join("node_modules")
                .join("beautiful-mermaid")
                .join("package.json"),
            "{}",
        )
        .expect("package manifest should be written");
        fs::write(
            helper_root.join("render_ascii.mjs"),
            "console.log('ascii');",
        )
        .expect("helper script should be written");

        let without_rich =
            discover_renderers_with_path(&PreviewConfig::default(), root.as_path(), None, None);
        assert_eq!(without_rich.ascii_helper_command, None);
        assert_eq!(without_rich.preferred_fallback(), MermaidDisplay::RawSource);

        let path_mmdc = root.join("bin").join("mmdc");
        write_fake_executable(&path_mmdc);
        let with_rich = discover_renderers_with_path(
            &PreviewConfig::default(),
            root.as_path(),
            Some(path_env(&[path_mmdc
                .parent()
                .expect("bin dir should exist")])),
            None,
        );
        assert_eq!(with_rich.rich_command.as_deref(), Some(path_mmdc.as_path()));
        assert_eq!(
            with_rich.ascii_helper_command.as_deref(),
            Some(helper_root.join("render_ascii.mjs").as_path())
        );
        assert_eq!(with_rich.preferred_fallback(), MermaidDisplay::Pending);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn discovery_finds_installed_release_helper_layout() {
        let root = make_temp_dir("grove-mermaid-discovery-installed-helper");
        let helper_dir = root.join("share").join("grove").join("mermaid");
        fs::create_dir_all(helper_dir.join("node_modules").join("beautiful-mermaid"))
            .expect("helper package dir should exist");
        fs::write(
            helper_dir
                .join("node_modules")
                .join("beautiful-mermaid")
                .join("package.json"),
            "{}",
        )
        .expect("package manifest should be written");
        fs::write(helper_dir.join("render_ascii.mjs"), "console.log('ascii');")
            .expect("helper script should be written");

        let path_mmdc = root.join("bin").join("mmdc");
        write_fake_executable(&path_mmdc);
        let fake_exe = root.join("bin").join("grove");
        write_fake_executable(&fake_exe);

        let discovery = discover_renderers_with_path(
            &PreviewConfig::default(),
            root.join("unrelated-source-root").as_path(),
            Some(path_env(&[path_mmdc
                .parent()
                .expect("bin dir should exist")])),
            Some(fake_exe.as_path()),
        );

        assert_eq!(discovery.rich_command.as_deref(), Some(path_mmdc.as_path()));
        assert_eq!(
            discovery.ascii_helper_command.as_deref(),
            Some(helper_dir.join("render_ascii.mjs").as_path())
        );
        assert_eq!(discovery.preferred_fallback(), MermaidDisplay::Pending);

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn render_request_falls_back_to_raw_source_when_graphics_are_disabled() {
        let request = MermaidRenderRequest {
            key: MermaidRenderKey {
                generation: PreviewGeneration(1),
                content_hash: 7,
            },
            source: MermaidSource {
                kind: MermaidSourceKind::NativeFile,
                block_index: None,
                total_blocks: 1,
                label: "Mermaid".to_string(),
                raw_source: "graph TD;A-->B;".to_string(),
            },
            discovery: MermaidRenderDiscovery {
                rich_command: Some(PathBuf::from("/tmp/mmdc")),
                ascii_helper_command: None,
            },
            graphics_enabled: false,
            timeout_ms: 50,
        };

        let response = render_request(request);
        assert_eq!(response.outcome, MermaidRenderOutcome::RawSource);
        assert!(
            response.status.contains("showing raw source"),
            "non-iTerm Mermaid rendering should degrade to raw source"
        );
    }

    #[test]
    fn render_png_with_mmdc_reads_png_bytes_from_a_real_png_output_path() {
        let root = make_temp_dir("grove-mermaid-mmdc-wrapper");
        let fake_mmdc = root.join("bin").join("mmdc");
        write_fake_mermaid_renderer(&fake_mmdc);

        let png = render_png_with_mmdc(&fake_mmdc, "graph TD\nA-->B\n", 1_000)
            .expect("renderer wrapper should read image bytes from the output path");

        assert_eq!(png, b"PNG".to_vec());

        fs::remove_dir_all(root).expect("temp root should be removed");
    }

    #[test]
    fn ascii_helper_help_exits_successfully() {
        let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tools")
            .join("mermaid")
            .join("render_ascii.mjs");
        let output = Command::new("node")
            .arg(&helper)
            .arg("--help")
            .output()
            .expect("node should run the mermaid helper");

        assert!(
            output.status.success(),
            "helper --help should exit successfully"
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
        assert!(
            stdout.contains("Usage: node tools/mermaid/render_ascii.mjs"),
            "helper --help should print usage to stdout"
        );
        assert!(
            output.stderr.is_empty(),
            "helper --help should not print an error"
        );
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

    fn write_fake_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir should exist");
        }
        fs::write(path, "#!/bin/sh\nexit 0\n").expect("fake executable should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path)
                .expect("metadata should load")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("permissions should update");
        }
    }

    fn write_fake_mermaid_renderer(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir should exist");
        }
        let script = r#"#!/bin/sh
output=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then
    shift
    output="$1"
    break
  fi
  shift
done
case "$output" in
  *.png) printf 'PNG' > "$output" ;;
  *) echo "output path must end with .png" >&2; exit 1 ;;
esac
"#;
        fs::write(path, script).expect("fake renderer should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path)
                .expect("metadata should load")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("permissions should update");
        }
    }

    fn path_env(paths: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(paths).expect("paths should join")
    }
}
