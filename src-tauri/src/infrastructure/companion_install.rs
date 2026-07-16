//! Managed Companion install under `%LocalAppData%/NinjaCrawler/Companion`.
//!
//! The desktop app can download the release ZIP into this folder so the
//! extension can apply it with `chrome.runtime.reload()` when the user has
//! loaded unpacked from this path (or switches Load unpacked to it once).

use crate::infrastructure::storage;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use zip::ZipArchive;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const ARCHIVE_ROOT: &str = "NinjaCrawler-Companion";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionInstallStatus {
    pub install_path: String,
    pub staged_version: Option<String>,
    pub available_version: String,
    pub update_ready: bool,
    pub download_url: String,
}

pub fn companion_install_dir() -> Result<PathBuf, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    Ok(layout.root.join("Companion"))
}

pub fn install_status(available_version: &str, download_url: &str) -> Result<CompanionInstallStatus, String> {
    let install_path = companion_install_dir()?;
    let staged_version = read_staged_version(&install_path);
    let update_ready = staged_version
        .as_deref()
        .is_some_and(|version| version == available_version);

    Ok(CompanionInstallStatus {
        install_path: install_path.display().to_string(),
        staged_version,
        available_version: available_version.to_string(),
        update_ready,
        download_url: download_url.to_string(),
    })
}

pub fn stage_update(available_version: &str, download_url: &str) -> Result<CompanionInstallStatus, String> {
    if available_version.trim().is_empty() {
        return Err("Available Companion version is unknown.".to_string());
    }
    if !download_url.starts_with("https://github.com/MetalDevOps/NinjaCrawler/") {
        return Err("Companion download URL is not trusted.".to_string());
    }

    let install_path = companion_install_dir()?;
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let cache_dir = layout.cache_root.join("companion-update");
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "Failed to create Companion update cache at '{}': {error}",
            cache_dir.display()
        )
    })?;

    let zip_path = cache_dir.join(format!("NinjaCrawler-Companion-{available_version}.zip"));
    download_to_file(download_url, &zip_path)?;

    let bytes = fs::read(&zip_path).map_err(|error| {
        format!(
            "Failed to read downloaded Companion archive '{}': {error}",
            zip_path.display()
        )
    })?;

    extract_companion_archive(&bytes, &install_path)?;

    let staged_version = read_staged_version(&install_path).ok_or_else(|| {
        format!(
            "Companion archive extracted to '{}' but manifest.json is missing or invalid.",
            install_path.display()
        )
    })?;

    if staged_version != available_version {
        return Err(format!(
            "Staged Companion version '{staged_version}' does not match available '{available_version}'."
        ));
    }

    install_status(available_version, download_url)
}

fn read_staged_version(install_path: &Path) -> Option<String> {
    let manifest_path = install_path.join("manifest.json");
    let raw = fs::read_to_string(manifest_path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    value
        .get("version")?
        .as_str()
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

fn download_to_file(url: &str, destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("Failed to prepare Companion download client: {error}"))?;

    let mut response = client
        .get(url)
        .header("User-Agent", "NinjaCrawler-companion-update")
        .header("Accept", "application/octet-stream")
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Failed to download Companion update: {error}"))?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let temporary = destination.with_extension("zip.partial");
    let mut file = File::create(&temporary).map_err(|error| {
        format!(
            "Failed to create temporary Companion archive '{}': {error}",
            temporary.display()
        )
    })?;
    io::copy(&mut response, &mut file).map_err(|error| {
        format!("Failed to write Companion archive '{}': {error}", temporary.display())
    })?;
    file.flush().map_err(|error| error.to_string())?;
    drop(file);

    if destination.exists() {
        let _ = fs::remove_file(destination);
    }
    fs::rename(&temporary, destination).map_err(|error| {
        format!(
            "Failed to finalize Companion archive '{}': {error}",
            destination.display()
        )
    })?;
    Ok(())
}

fn extract_companion_archive(bytes: &[u8], install_path: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Failed to open Companion ZIP: {error}"))?;

    let parent = install_path
        .parent()
        .ok_or_else(|| "Companion install path has no parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;

    let staging = parent.join(format!(
        ".Companion-staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

    let result = (|| -> Result<(), String> {
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| format!("Failed to read Companion ZIP entry: {error}"))?;
            let Some(enclosed) = entry.enclosed_name().map(PathBuf::from) else {
                continue;
            };
            let relative = strip_archive_root(&enclosed);
            if relative.as_os_str().is_empty() {
                continue;
            }
            if relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
            {
                return Err(format!(
                    "Refusing unsafe Companion ZIP path '{}'.",
                    enclosed.display()
                ));
            }

            let output_path = staging.join(&relative);
            if entry.is_dir() {
                fs::create_dir_all(&output_path).map_err(|error| error.to_string())?;
                continue;
            }

            if let Some(parent_dir) = output_path.parent() {
                fs::create_dir_all(parent_dir).map_err(|error| error.to_string())?;
            }
            let mut output = File::create(&output_path).map_err(|error| {
                format!(
                    "Failed to create '{}' while extracting Companion: {error}",
                    output_path.display()
                )
            })?;
            io::copy(&mut entry, &mut output).map_err(|error| {
                format!(
                    "Failed to extract '{}' from Companion ZIP: {error}",
                    output_path.display()
                )
            })?;
        }

        let manifest = staging.join("manifest.json");
        if !manifest.is_file() {
            return Err(
                "Companion ZIP does not contain manifest.json at the package root.".to_string(),
            );
        }

        if install_path.exists() {
            fs::remove_dir_all(install_path).map_err(|error| {
                format!(
                    "Failed to replace existing Companion install at '{}': {error}",
                    install_path.display()
                )
            })?;
        }
        fs::rename(&staging, install_path).map_err(|error| {
            format!(
                "Failed to activate Companion install at '{}': {error}",
                install_path.display()
            )
        })?;
        Ok(())
    })();

    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn strip_archive_root(path: &Path) -> PathBuf {
    let mut components = path.components();
    if let Some(Component::Normal(first)) = components.next() {
        if first == ARCHIVE_ROOT {
            return components.collect();
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    #[test]
    fn strips_known_archive_root() {
        let stripped = strip_archive_root(Path::new("NinjaCrawler-Companion/manifest.json"));
        assert_eq!(stripped, PathBuf::from("manifest.json"));
    }

    #[test]
    fn extract_writes_manifest_under_install_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let install = temp.path().join("Companion");

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("NinjaCrawler-Companion/manifest.json", options)
                .expect("start file");
            zip.write_all(br#"{"version":"0.99.0"}"#).expect("write");
            zip.start_file("NinjaCrawler-Companion/popup.html", options)
                .expect("start popup");
            zip.write_all(b"<html></html>").expect("write popup");
            zip.finish().expect("finish zip");
        }

        extract_companion_archive(buffer.get_ref(), &install).expect("extract");
        let version = read_staged_version(&install);
        assert_eq!(version.as_deref(), Some("0.99.0"));
        assert!(install.join("popup.html").is_file());
    }
}
