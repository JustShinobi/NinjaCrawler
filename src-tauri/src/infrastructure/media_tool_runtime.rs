use crate::infrastructure::storage::{ensure_workspace_layout, StorageLayout};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use zip::ZipArchive;

pub(crate) const FFMPEG_VERSION: &str = "8.1.2";
const FFMPEG_ASSET_URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/8.1.2/ffmpeg-8.1.2-essentials_build.zip";
const FFMPEG_ASSET_SHA256: &str =
    "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec";
const FFMPEG_MAX_ARCHIVE_BYTES: u64 = 160 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct MediaToolStatus {
    pub status: String,
    pub available: bool,
    pub source: Option<String>,
    pub version: Option<String>,
    pub install_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct MediaToolPaths {
    pub ffmpeg: PathBuf,
    pub bin_dir: Option<PathBuf>,
    pub source: String,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct InstalledManifest {
    version: String,
    asset_url: String,
    asset_sha256: String,
    bin_relative_path: String,
}

pub(crate) fn status() -> MediaToolStatus {
    match resolve() {
        Some(paths) => MediaToolStatus {
            status: "ready".to_string(),
            available: true,
            source: Some(paths.source),
            version: paths.version,
            install_path: paths
                .bin_dir
                .map(|path| path.to_string_lossy().to_string()),
            error: None,
        },
        None if !cfg!(all(windows, target_arch = "x86_64")) => MediaToolStatus {
            status: "not_installed".to_string(),
            available: false,
            source: None,
            version: None,
            install_path: None,
            error: Some(
                "Automatic FFmpeg installation currently supports Windows x64 only."
                    .to_string(),
            ),
        },
        None => MediaToolStatus {
            status: "not_installed".to_string(),
            available: false,
            source: None,
            version: None,
            install_path: None,
            error: None,
        },
    }
}

pub(crate) fn resolve() -> Option<MediaToolPaths> {
    let cache = resolved_cache();
    if let Ok(state) = cache.lock() {
        if let Some(paths) = state.as_ref() {
            return Some(paths.clone());
        }
    }
    let resolved = resolve_system().or_else(resolve_managed);
    if let (Some(paths), Ok(mut state)) = (resolved.as_ref(), cache.lock()) {
        *state = Some(paths.clone());
    }
    resolved
}

pub(crate) fn ffmpeg_executable() -> Option<PathBuf> {
    resolve().map(|paths| paths.ffmpeg)
}

pub(crate) fn configure_tool_path(command: &mut Command) {
    let Some(paths) = resolve() else {
        return;
    };
    let Some(bin_dir) = paths.bin_dir else {
        return;
    };
    let mut values = vec![bin_dir];
    if let Some(existing) = env::var_os("PATH") {
        values.extend(env::split_paths(&existing));
    }
    if let Ok(joined) = env::join_paths(values) {
        command.env("PATH", joined);
    }
}

pub(crate) fn install() -> Result<MediaToolStatus, String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("Automatic FFmpeg installation currently supports Windows x64 only.".to_string());
    }
    let layout = ensure_workspace_layout().map_err(|error| error.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("NinjaCrawler managed media tools")
        .build()
        .map_err(|error| format!("Failed to prepare FFmpeg download: {error}"))?;
    let response = client
        .get(FFMPEG_ASSET_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Failed to download the FFmpeg runtime: {error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > FFMPEG_MAX_ARCHIVE_BYTES)
    {
        return Err("The FFmpeg archive is larger than the approved runtime package.".to_string());
    }
    let mut bytes = Vec::new();
    response
        .take(FFMPEG_MAX_ARCHIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Failed to read the FFmpeg archive: {error}"))?;
    if bytes.len() as u64 > FFMPEG_MAX_ARCHIVE_BYTES {
        return Err("The FFmpeg archive exceeded the approved download size.".to_string());
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(FFMPEG_ASSET_SHA256) {
        return Err(format!(
            "FFmpeg archive failed verification. Expected {FFMPEG_ASSET_SHA256}, received {digest}."
        ));
    }

    let install_root = install_root(&layout);
    let staging = install_root.with_extension(format!("staging-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    if let Err(error) = extract_archive(&bytes, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let Some((ffmpeg, ffprobe)) = find_tool_pair(&staging) else {
        let _ = fs::remove_dir_all(&staging);
        return Err("The verified FFmpeg archive does not contain ffmpeg.exe and ffprobe.exe in the same directory.".to_string());
    };
    let bin_dir = ffmpeg
        .parent()
        .ok_or_else(|| "The extracted FFmpeg path is invalid.".to_string())?;
    if !command_success(&ffmpeg, &["-version"]) || !command_success(&ffprobe, &["-version"]) {
        let _ = fs::remove_dir_all(&staging);
        return Err("The verified FFmpeg runtime could not be executed on this computer.".to_string());
    }
    let relative_bin = bin_dir
        .strip_prefix(&staging)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .to_string();
    let manifest = InstalledManifest {
        version: FFMPEG_VERSION.to_string(),
        asset_url: FFMPEG_ASSET_URL.to_string(),
        asset_sha256: FFMPEG_ASSET_SHA256.to_string(),
        bin_relative_path: relative_bin,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(staging.join("ninjacrawler-runtime.json"), manifest_json)
        .map_err(|error| error.to_string())?;
    if let Some(parent) = install_root.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let backup = install_root.with_extension(format!("backup-{}", uuid::Uuid::new_v4()));
    if install_root.exists() {
        fs::rename(&install_root, &backup)
            .map_err(|error| format!("Failed to stage the previous FFmpeg runtime: {error}"))?;
    }
    if let Err(error) = fs::rename(&staging, &install_root) {
        if backup.exists() {
            let _ = fs::rename(&backup, &install_root);
        }
        return Err(format!("Failed to activate the FFmpeg runtime: {error}"));
    }
    if backup.exists() {
        let _ = fs::remove_dir_all(backup);
    }
    invalidate_cache();
    Ok(status())
}

fn resolved_cache() -> &'static Mutex<Option<MediaToolPaths>> {
    static CACHE: OnceLock<Mutex<Option<MediaToolPaths>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn invalidate_cache() {
    if let Ok(mut state) = resolved_cache().lock() {
        *state = None;
    }
}

fn resolve_system() -> Option<MediaToolPaths> {
    let ffmpeg = PathBuf::from(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    let ffprobe = PathBuf::from(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
    if !command_success(&ffmpeg, &["-version"]) || !command_success(&ffprobe, &["-version"]) {
        return None;
    }
    Some(MediaToolPaths {
        version: command_version(&ffmpeg),
        ffmpeg,
        bin_dir: None,
        source: "system".to_string(),
    })
}

fn resolve_managed() -> Option<MediaToolPaths> {
    let layout = ensure_workspace_layout().ok()?;
    let root = install_root(&layout);
    let manifest: InstalledManifest =
        serde_json::from_str(&fs::read_to_string(root.join("ninjacrawler-runtime.json")).ok()?)
            .ok()?;
    if manifest.version != FFMPEG_VERSION
        || !manifest.asset_sha256.eq_ignore_ascii_case(FFMPEG_ASSET_SHA256)
    {
        return None;
    }
    let bin_dir = root.join(manifest.bin_relative_path);
    let ffmpeg = bin_dir.join("ffmpeg.exe");
    let ffprobe = bin_dir.join("ffprobe.exe");
    if !ffmpeg.is_file()
        || !ffprobe.is_file()
        || !command_success(&ffmpeg, &["-version"])
        || !command_success(&ffprobe, &["-version"])
    {
        return None;
    }
    Some(MediaToolPaths {
        version: command_version(&ffmpeg).or_else(|| Some(FFMPEG_VERSION.to_string())),
        ffmpeg,
        bin_dir: Some(bin_dir),
        source: "managed".to_string(),
    })
}

fn install_root(layout: &StorageLayout) -> PathBuf {
    layout
        .connectors_root
        .join("media-tools")
        .join("ffmpeg")
        .join(FFMPEG_VERSION)
}

fn find_tool_pair(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).ok()?;
        let mut ffmpeg = None;
        let mut ffprobe = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            match path.file_name().and_then(|value| value.to_str()) {
                Some(value) if value.eq_ignore_ascii_case("ffmpeg.exe") => ffmpeg = Some(path),
                Some(value) if value.eq_ignore_ascii_case("ffprobe.exe") => ffprobe = Some(path),
                _ => {}
            }
        }
        if let (Some(ffmpeg), Some(ffprobe)) = (ffmpeg, ffprobe) {
            return Some((ffmpeg, ffprobe));
        }
    }
    None
}

fn extract_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Failed to open the FFmpeg archive: {error}"))?;
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative) = member.enclosed_name() else {
            return Err("The FFmpeg archive contains an unsafe path.".to_string());
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

fn command_success(executable: &Path, args: &[&str]) -> bool {
    let mut command = Command::new(executable);
    command.args(args);
    configure_background_command(&mut command);
    command
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_version(executable: &Path) -> Option<String> {
    let mut command = Command::new(executable);
    command.arg("-version");
    configure_background_command(&mut command);
    let output = command.output().ok()?;
    let line = String::from_utf8_lossy(&output.stdout).lines().next()?.to_string();
    line.strip_prefix("ffmpeg version ")
        .and_then(|value| value.split_whitespace().next())
        .map(str::to_string)
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_digest_is_sha256() {
        assert_eq!(FFMPEG_ASSET_SHA256.len(), 64);
        assert!(FFMPEG_ASSET_SHA256
            .bytes()
            .all(|value| value.is_ascii_hexdigit()));
    }

    #[test]
    fn extracted_tools_are_discovered_below_the_archive_root() {
        let temp = tempfile::tempdir().expect("temp");
        let bin = temp.path().join("ffmpeg-build").join("bin");
        fs::create_dir_all(&bin).expect("bin");
        fs::write(bin.join("ffmpeg.exe"), b"ffmpeg").expect("ffmpeg");
        fs::write(bin.join("ffprobe.exe"), b"ffprobe").expect("ffprobe");
        let (ffmpeg, ffprobe) = find_tool_pair(temp.path()).expect("tool pair");
        assert_eq!(ffmpeg.parent(), ffprobe.parent());
    }
}
