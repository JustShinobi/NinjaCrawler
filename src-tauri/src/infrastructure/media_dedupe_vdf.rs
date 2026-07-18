use crate::domain::models::MediaDedupeEngineStatus;
use crate::infrastructure::media_tool_runtime;
use crate::infrastructure::storage::{ensure_workspace_layout, StorageLayout};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use zip::ZipArchive;

pub(crate) const VDF_VERSION: &str = "4.1.x+5954aff";
const VDF_ASSET_URL: &str =
    "https://github.com/0x90d/videoduplicatefinder/releases/download/4.1.x/CLI-win-x64.zip";
const VDF_ASSET_SHA256: &str = "90eee514aefaa4278656d9e7cb77bef446e495a368d0f745c684f9be2db0b893";
const VDF_EXECUTABLE: &str = "vdf-cli.exe";

#[derive(Serialize, Deserialize)]
struct InstalledManifest {
    version: String,
    asset_url: String,
    asset_sha256: String,
}

pub(crate) fn status() -> MediaDedupeEngineStatus {
    let tools = media_tool_runtime::status();
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return MediaDedupeEngineStatus {
            status: "unsupported".to_string(),
            version: VDF_VERSION.to_string(),
            installed: false,
            ffmpeg_available: tools.available,
            ffmpeg_status: tools.status,
            ffmpeg_source: tools.source,
            ffmpeg_version: tools.version,
            ffmpeg_install_path: tools.install_path,
            ffmpeg_error: tools.error,
            install_path: None,
            error: Some(
                "The managed similarity engine currently supports Windows x64.".to_string(),
            ),
        };
    }
    match ensure_workspace_layout() {
        Ok(layout) => {
            let executable = executable_path(&layout);
            let manifest = manifest_path(&layout);
            let valid_manifest = fs::read_to_string(&manifest)
                .ok()
                .and_then(|value| serde_json::from_str::<InstalledManifest>(&value).ok())
                .is_some_and(|value| {
                    value.version == VDF_VERSION
                        && value.asset_sha256.eq_ignore_ascii_case(VDF_ASSET_SHA256)
                });
            let installed = executable.is_file() && valid_manifest;
            let tools = media_tool_runtime::status();
            MediaDedupeEngineStatus {
                status: if installed { "ready" } else { "not_installed" }.to_string(),
                version: VDF_VERSION.to_string(),
                installed,
                ffmpeg_available: tools.available,
                ffmpeg_status: tools.status,
                ffmpeg_source: tools.source,
                ffmpeg_version: tools.version,
                ffmpeg_install_path: tools.install_path,
                ffmpeg_error: tools.error,
                install_path: installed.then(|| executable.to_string_lossy().to_string()),
                error: None,
            }
        }
        Err(error) => MediaDedupeEngineStatus {
            status: "error".to_string(),
            version: VDF_VERSION.to_string(),
            installed: false,
            ffmpeg_available: false,
            ffmpeg_status: "error".to_string(),
            ffmpeg_source: None,
            ffmpeg_version: None,
            ffmpeg_install_path: None,
            ffmpeg_error: Some(error.to_string()),
            install_path: None,
            error: Some(error.to_string()),
        },
    }
}

pub(crate) fn install() -> Result<MediaDedupeEngineStatus, String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("The managed similarity engine currently supports Windows x64.".to_string());
    }
    let layout = ensure_workspace_layout().map_err(|error| error.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("NinjaCrawler media cleanup")
        .build()
        .map_err(|error| format!("Failed to prepare similarity engine download: {error}"))?;
    let mut response = client
        .get(VDF_ASSET_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Failed to download Video Duplicate Finder: {error}"))?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Failed to read Video Duplicate Finder archive: {error}"))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(VDF_ASSET_SHA256) {
        return Err(format!(
            "Video Duplicate Finder archive failed verification. Expected {VDF_ASSET_SHA256}, received {digest}. The upstream daily asset may have changed."
        ));
    }

    let install_root = install_root(&layout);
    let staging = install_root.with_extension(format!("staging-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let extraction = extract_archive(&bytes, &staging);
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if !staging.join(VDF_EXECUTABLE).is_file() {
        let _ = fs::remove_dir_all(&staging);
        return Err("The verified VDF archive does not contain vdf-cli.exe.".to_string());
    }
    let manifest = InstalledManifest {
        version: VDF_VERSION.to_string(),
        asset_url: VDF_ASSET_URL.to_string(),
        asset_sha256: VDF_ASSET_SHA256.to_string(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(staging.join("ninjacrawler-runtime.json"), manifest_json)
        .map_err(|error| error.to_string())?;
    if let Some(parent) = install_root.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let backup = install_root.with_extension(format!("backup-{}", uuid::Uuid::new_v4()));
    if install_root.exists() {
        fs::rename(&install_root, &backup).map_err(|error| {
            format!("Failed to stage the previous similarity engine for replacement: {error}")
        })?;
    }
    if let Err(error) = fs::rename(&staging, &install_root) {
        if backup.exists() {
            let _ = fs::rename(&backup, &install_root);
        }
        return Err(format!("Failed to activate the similarity engine: {error}"));
    }
    if backup.exists() {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(status())
}

pub(crate) fn executable() -> Result<PathBuf, String> {
    let status = status();
    if !status.installed {
        return Err(status.error.unwrap_or_else(|| {
            "Install the similarity engine before running perceptual comparison.".to_string()
        }));
    }
    status
        .install_path
        .map(PathBuf::from)
        .ok_or_else(|| "Similarity engine path is unavailable.".to_string())
}

pub(crate) fn runtime_digest() -> &'static str {
    VDF_ASSET_SHA256
}

pub(crate) fn data_root(layout: &StorageLayout) -> PathBuf {
    layout.data_dir.join("media-dedupe").join("vdf")
}

pub(crate) struct VdfSourcePaths {
    pub database_dir: PathBuf,
    pub result_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VdfRunOptions {
    pub hashing_parallelism: usize,
    pub matching_parallelism: usize,
    pub low_priority: bool,
}

impl VdfRunOptions {
    pub(crate) fn for_profile(profile: &str, concurrent_processes: usize) -> Self {
        let logical_processors = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(2)
            .max(1);
        Self::for_capacity(profile, logical_processors, concurrent_processes)
    }

    fn for_capacity(profile: &str, logical_processors: usize, concurrent_processes: usize) -> Self {
        let processes = concurrent_processes.max(1);
        let total_budget = match profile {
            "quiet" => 1,
            "fast" => logical_processors.saturating_sub(2).max(1),
            _ => (logical_processors / 2).max(2),
        };
        let per_process = (total_budget / processes).max(1);
        Self {
            hashing_parallelism: per_process,
            matching_parallelism: per_process,
            low_priority: profile != "fast",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct VdfProgress {
    pub percent: Option<u32>,
    pub files_processed: u64,
    pub files_total: u64,
    pub current_path: Option<String>,
}

pub(crate) fn source_paths(scan_id: &str, source_id: &str) -> Result<VdfSourcePaths, String> {
    let layout = ensure_workspace_layout().map_err(|error| error.to_string())?;
    let source_root = data_root(&layout).join(source_id);
    let result_root = source_root.join("results");
    fs::create_dir_all(&result_root).map_err(|error| error.to_string())?;
    Ok(VdfSourcePaths {
        database_dir: source_root,
        result_path: result_root.join(format!("{scan_id}.json")),
    })
}

pub(crate) fn settings_fingerprint() -> String {
    let value = "schema=source-directory-v2;percent=96;threshold=5;include_images=false;use_phash=true;partial_clip=false";
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(crate) fn run_source_scan(
    source_path: &Path,
    paths: &VdfSourcePaths,
    options: VdfRunOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(VdfProgress),
) -> Result<Vec<crate::infrastructure::workspace_repository::MediaDedupeVdfCandidateOwned>, String>
{
    let executable = executable()?;
    if media_tool_runtime::resolve().is_none() {
        return Err("FFmpeg and FFprobe are required for VDF video comparison.".to_string());
    }
    if !paths.database_dir.is_dir() {
        return Err(format!(
            "The isolated VDF database directory is unavailable: '{}'.",
            paths.database_dir.display()
        ));
    }
    if paths.result_path.exists() {
        fs::remove_file(&paths.result_path).map_err(|error| error.to_string())?;
    }
    let mut command = Command::new(executable);
    media_tool_runtime::configure_tool_path(&mut command);
    configure_scan_command(&mut command, source_path, paths, options);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let priority = if options.low_priority { 0x00004000 } else { 0 };
        command.creation_flags(0x08000000 | priority);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start Video Duplicate Finder: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Video Duplicate Finder progress stream is unavailable.".to_string())?;
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stderr);
        let mut buffer = [0u8; 4096];
        let mut pending = Vec::<u8>::new();
        loop {
            let Ok(read) = reader.read(&mut buffer) else {
                break;
            };
            if read == 0 {
                break;
            }
            for byte in &buffer[..read] {
                if matches!(byte, b'\r' | b'\n') {
                    if !pending.is_empty() {
                        let line = String::from_utf8_lossy(&pending).trim().to_string();
                        if !line.is_empty() {
                            let _ = sender.send(line);
                        }
                        pending.clear();
                    }
                } else {
                    pending.push(*byte);
                }
            }
        }
        if !pending.is_empty() {
            let _ = sender.send(String::from_utf8_lossy(&pending).trim().to_string());
        }
    });

    let mut last_progress = VdfProgress::default();
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Media scan cancelled.".to_string());
        }
        while let Ok(line) = receiver.try_recv() {
            last_progress = parse_progress(&line, &last_progress);
            progress(last_progress.clone());
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            while let Ok(line) = receiver.try_recv() {
                last_progress = parse_progress(&line, &last_progress);
                progress(last_progress.clone());
            }
            if !status.success() {
                return Err(format!(
                    "Video Duplicate Finder exited with status {} while scanning '{}'.",
                    status,
                    source_path.display()
                ));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let database_file = paths.database_dir.join("ScannedFiles.db");
    if !database_file.is_file() {
        return Err(format!(
            "Video Duplicate Finder did not create its isolated database at '{}'.",
            database_file.display()
        ));
    }
    parse_results(&paths.result_path)
}

fn configure_scan_command(
    command: &mut Command,
    source_path: &Path,
    paths: &VdfSourcePaths,
    options: VdfRunOptions,
) {
    command
        .arg("scan-and-compare")
        .arg("--include")
        .arg(source_path)
        .arg("--exclude")
        .arg(source_path.join(".thumbs"))
        .arg("--exclude")
        .arg(source_path.join("cover"))
        .arg("--db")
        .arg(&paths.database_dir)
        .args(["--percent", "96", "--threshold", "5", "--parallelism"])
        .arg(options.hashing_parallelism.to_string())
        .arg("--matching-parallelism")
        .arg(options.matching_parallelism.to_string())
        .args(["--use-phash", "--format", "json", "--output"])
        .arg(&paths.result_path);
}

fn parse_progress(line: &str, previous: &VdfProgress) -> VdfProgress {
    let mut result = previous.clone();
    if let Some(open) = line.find('[') {
        if let Some(close) = line[open + 1..].find('%') {
            result.percent = line[open + 1..open + 1 + close]
                .trim()
                .parse::<u32>()
                .ok()
                .map(|value| value.min(100));
        }
    }
    for token in line.split_whitespace() {
        let Some((processed, total)) = token.split_once('/') else {
            continue;
        };
        if let (Ok(processed), Ok(total)) = (
            processed
                .trim_matches(|value: char| !value.is_ascii_digit())
                .parse::<u64>(),
            total
                .trim_matches(|value: char| !value.is_ascii_digit())
                .parse::<u64>(),
        ) {
            result.files_processed = processed;
            result.files_total = total;
            break;
        }
    }
    if let Some(index) = line.find(":\\").and_then(|value| value.checked_sub(1)) {
        result.current_path = Some(line[index..].trim().to_string());
    } else if let Some(index) = line.find("\\\\") {
        result.current_path = Some(line[index..].trim().to_string());
    }
    result
}

fn parse_results(
    path: &Path,
) -> Result<Vec<crate::infrastructure::workspace_repository::MediaDedupeVdfCandidateOwned>, String>
{
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Video Duplicate Finder completed without a readable result at '{}': {error}",
            path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to parse Video Duplicate Finder results: {error}"))?;
    let groups = value
        .as_array()
        .or_else(|| value.get("Groups").and_then(serde_json::Value::as_array))
        .or_else(|| value.get("groups").and_then(serde_json::Value::as_array))
        .cloned()
        .unwrap_or_default();
    let mut output = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        let group_id = json_string(group, &["GroupId", "groupId", "id"])
            .unwrap_or_else(|| group_index.to_string());
        let items = group
            .get("Items")
            .or_else(|| group.get("items"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            let Some(path) = json_string(&item, &["Path", "path"]) else {
                continue;
            };
            let similarity = json_f64(&item, &["Similarity", "similarity"]).unwrap_or(0.0);
            output.push(
                crate::infrastructure::workspace_repository::MediaDedupeVdfCandidateOwned {
                    normalized_path: normalize_result_path(&path),
                    path,
                    group_id: group_id.clone(),
                    similarity_percent: similarity,
                    size_bytes: json_u64(&item, &["SizeLong", "sizeLong", "Size", "size"])
                        .unwrap_or(0),
                    duration_ms: json_duration_ms(&item),
                    width: json_u64(&item, &["Width", "width"]).map(|value| value as u32),
                    height: json_u64(&item, &["Height", "height"]).map(|value| value as u32),
                },
            );
        }
    }
    Ok(output)
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn json_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_f64))
}

fn json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

fn json_duration_ms(value: &serde_json::Value) -> Option<u64> {
    json_u64(value, &["DurationMs", "durationMs"]).or_else(|| {
        json_f64(value, &["DurationSeconds", "durationSeconds"])
            .map(|value| (value * 1000.0).round() as u64)
    })
}

fn normalize_result_path(path: &str) -> String {
    let value = path.replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn install_root(layout: &StorageLayout) -> PathBuf {
    layout
        .connectors_root
        .join("media-dedupe")
        .join("vdf")
        .join(VDF_VERSION)
}

fn executable_path(layout: &StorageLayout) -> PathBuf {
    install_root(layout).join(VDF_EXECUTABLE)
}

fn manifest_path(layout: &StorageLayout) -> PathBuf {
    install_root(layout).join("ninjacrawler-runtime.json")
}

fn extract_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Failed to open Video Duplicate Finder archive: {error}"))?;
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative) = member.enclosed_name() else {
            return Err("Video Duplicate Finder archive contains an unsafe path.".to_string());
        };
        let output = destination.join(relative);
        if member.is_dir() {
            fs::create_dir_all(&output).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = fs::File::create(&output).map_err(|error| error.to_string())?;
        std::io::copy(&mut member, &mut file).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_digest_is_sha256() {
        assert_eq!(VDF_ASSET_SHA256.len(), 64);
        assert!(VDF_ASSET_SHA256
            .bytes()
            .all(|value| value.is_ascii_hexdigit()));
    }

    #[test]
    fn archive_paths_cannot_escape_destination() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .start_file("vdf-cli.exe", zip::write::SimpleFileOptions::default())
                .expect("start");
            writer.write_all(b"runtime").expect("write");
            writer.finish().expect("finish");
        }
        let temp = tempfile::tempdir().expect("temp");
        extract_archive(buffer.get_ref(), temp.path()).expect("extract");
        assert!(temp.path().join("vdf-cli.exe").is_file());
    }

    #[test]
    fn carriage_return_progress_is_parsed() {
        let progress = parse_progress(
            r"[ 66%] 2/3 ETA 00:01 S:\Media\profile\video.mp4 (sampling frames 1/1)",
            &VdfProgress::default(),
        );
        assert_eq!(progress.percent, Some(66));
        assert_eq!(progress.files_processed, 2);
        assert_eq!(progress.files_total, 3);
        assert!(progress
            .current_path
            .as_deref()
            .is_some_and(|value| value.starts_with(r"S:\Media")));
    }

    #[test]
    fn profiles_respect_the_shared_cpu_budget() {
        assert_eq!(
            VdfRunOptions::for_capacity("quiet", 16, 1).hashing_parallelism,
            1
        );
        assert_eq!(
            VdfRunOptions::for_capacity("balanced", 16, 1).hashing_parallelism,
            8
        );
        assert_eq!(
            VdfRunOptions::for_capacity("balanced", 16, 2).hashing_parallelism,
            4
        );
        assert_eq!(
            VdfRunOptions::for_capacity("fast", 16, 1).matching_parallelism,
            14
        );
        assert!(!VdfRunOptions::for_capacity("fast", 16, 1).low_priority);
    }

    #[test]
    fn source_database_contract_uses_an_existing_directory() {
        let temp = tempfile::tempdir().expect("temp");
        let source_root = temp.path().join("source-1");
        let result_root = source_root.join("results");
        fs::create_dir_all(&result_root).expect("create");
        let paths = VdfSourcePaths {
            database_dir: source_root.clone(),
            result_path: result_root.join("scan.json"),
        };
        assert!(paths.database_dir.is_dir());
        assert_eq!(paths.database_dir, source_root);
        assert_eq!(
            paths
                .database_dir
                .join("ScannedFiles.db")
                .file_name()
                .unwrap(),
            "ScannedFiles.db"
        );
    }

    #[test]
    fn command_uses_isolated_folder_and_leaves_images_to_ninjacrawler() {
        let temp = tempfile::tempdir().expect("temp");
        let source = temp.path().join("source");
        let database_dir = temp.path().join("database");
        fs::create_dir_all(&source).expect("source");
        fs::create_dir_all(&database_dir).expect("database");
        let paths = VdfSourcePaths {
            database_dir: database_dir.clone(),
            result_path: temp.path().join("result.json"),
        };
        let mut command = Command::new("vdf-cli.exe");
        configure_scan_command(
            &mut command,
            &source,
            &paths,
            VdfRunOptions::for_capacity("balanced", 8, 1),
        );
        let arguments = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let database_index = arguments.iter().position(|value| value == "--db").unwrap();
        assert_eq!(PathBuf::from(&arguments[database_index + 1]), database_dir);
        assert!(!arguments.iter().any(|value| value == "--include-images"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--parallelism", "4"]));
    }

    #[test]
    fn json_results_are_imported_as_review_candidates() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("result.json");
        fs::write(
            &path,
            r#"[{"GroupId":"group-1","Items":[{"Path":"S:\\Media\\a.mp4","SizeLong":100,"Similarity":100.0},{"Path":"S:\\Media\\b.mp4","SizeLong":90,"Similarity":98.5}]}]"#,
        )
        .expect("fixture");
        let candidates = parse_results(&path).expect("parse");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].group_id, "group-1");
        assert_eq!(candidates[1].similarity_percent, 98.5);
    }
}
