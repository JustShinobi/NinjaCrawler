use std::time::Duration;

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;

use crate::domain::models::{AppBuildChannel, AppBuildInfo, AppUpdateStatus};

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/JustShinobi/NinjaCrawler/releases/latest";
const RELEASE_URL_PREFIX: &str = "https://github.com/JustShinobi/NinjaCrawler/releases/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Placeholder that ships in `tauri.conf.json` until the operator generates a
/// real minisign key pair with `cargo tauri signer generate`. When the pubkey
/// is still this placeholder the install flow fails gracefully instead of
/// panicking, and the lightweight GitHub check keeps working unchanged.
const UPDATER_PUBKEY_PLACEHOLDER: &str = "TAURI_UPDATER_PUBKEY_PLACEHOLDER";

/// Event emitted to the frontend while an update download/install is running.
pub const APP_UPDATE_PROGRESS_EVENT: &str = "runtime://app-update-progress";

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
}

pub fn build_info() -> AppBuildInfo {
    let channel = match env!("NINJACRAWLER_BUILD_CHANNEL") {
        "release" => AppBuildChannel::Release,
        _ => AppBuildChannel::Development,
    };
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit_sha = env!("NINJACRAWLER_BUILD_SHA").to_string();
    let dirty = env!("NINJACRAWLER_BUILD_DIRTY") == "true";
    let display_version = match channel {
        AppBuildChannel::Release => format!("v{version}"),
        AppBuildChannel::Development => {
            format!("Dev {commit_sha}{}", if dirty { "-dirty" } else { "" })
        }
    };

    AppBuildInfo {
        version,
        commit_sha,
        dirty,
        channel,
        display_version,
    }
}

pub fn check_app_update() -> Result<AppUpdateStatus, String> {
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("Failed to prepare the update request: {error}"))?;
    let response = client
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .header("User-Agent", "NinjaCrawler-update-check")
        .send()
        .map_err(|error| format!("Could not check for updates: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub returned an error while checking for updates: {error}"))?;
    let release = response
        .json::<GitHubRelease>()
        .map_err(|error| format!("GitHub returned an invalid release response: {error}"))?;

    status_from_release(build_info(), release)
}

fn strict_release_version(tag: &str) -> Result<Version, String> {
    let raw = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("Latest release tag '{tag}' does not use the vX.Y.Z format."))?;
    let version = Version::parse(raw)
        .map_err(|_| format!("Latest release tag '{tag}' does not use the vX.Y.Z format."))?;
    if !version.pre.is_empty()
        || !version.build.is_empty()
        || format!("v{}.{}.{}", version.major, version.minor, version.patch) != tag
    {
        return Err(format!(
            "Latest release tag '{tag}' does not use the vX.Y.Z format."
        ));
    }
    Ok(version)
}

fn status_from_release(
    build: AppBuildInfo,
    release: GitHubRelease,
) -> Result<AppUpdateStatus, String> {
    let latest = strict_release_version(&release.tag_name)?;
    if !release.html_url.starts_with(RELEASE_URL_PREFIX) {
        return Err("GitHub returned an unexpected release URL.".to_string());
    }
    let current = Version::parse(&build.version).map_err(|error| {
        format!(
            "Current app version '{}' is invalid: {error}",
            build.version
        )
    })?;
    let update_available = build.channel == AppBuildChannel::Release && latest > current;

    Ok(AppUpdateStatus {
        build,
        latest_version: latest.to_string(),
        release_url: release.html_url,
        published_at: release.published_at,
        update_available,
    })
}

/// Progress payload emitted while an update is downloaded and installed.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateProgress {
    /// One of `downloading`, `installing`, `done`.
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    percent: Option<u32>,
}

/// Downloads and installs the latest release via `tauri-plugin-updater`, then
/// relaunches the app. Progress is streamed to the frontend through the
/// [`APP_UPDATE_PROGRESS_EVENT`] event.
///
/// The lightweight [`check_app_update`] flow above stays independent: it talks
/// to the GitHub API directly for the "update available" banner. This function
/// is the heavier "Install update" path that requires a configured signing
/// public key. If the key is still the placeholder (or the updater is otherwise
/// not configured) it fails gracefully with a clear message and never panics.
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;

    ensure_updater_configured(&app)?;

    let updater = app
        .updater()
        .map_err(|error| format!("The auto-updater is not configured: {error}"))?;

    let update = updater
        .check()
        .await
        .map_err(|error| format!("Could not check for updates: {error}"))?
        .ok_or_else(|| "No update is available to install.".to_string())?;

    let downloaded = Arc::new(AtomicU64::new(0));
    let progress_app = app.clone();
    let progress_downloaded = Arc::clone(&downloaded);

    update
        .download_and_install(
            move |chunk_length, content_length| {
                let total = progress_downloaded
                    .fetch_add(chunk_length as u64, Ordering::Relaxed)
                    + chunk_length as u64;
                let percent = content_length
                    .filter(|len| *len > 0)
                    .map(|len| ((total.min(len) * 100) / len) as u32);
                let _ = progress_app.emit(
                    APP_UPDATE_PROGRESS_EVENT,
                    AppUpdateProgress {
                        phase: "downloading",
                        downloaded: total,
                        total: content_length,
                        percent,
                    },
                );
            },
            {
                let finish_app = app.clone();
                move || {
                    let _ = finish_app.emit(
                        APP_UPDATE_PROGRESS_EVENT,
                        AppUpdateProgress {
                            phase: "installing",
                            downloaded: 0,
                            total: None,
                            percent: Some(100),
                        },
                    );
                }
            },
        )
        .await
        .map_err(|error| format!("Failed to install the update: {error}"))?;

    let _ = app.emit(
        APP_UPDATE_PROGRESS_EVENT,
        AppUpdateProgress {
            phase: "done",
            downloaded: 0,
            total: None,
            percent: Some(100),
        },
    );

    // On success the new version is staged; relaunching swaps it in.
    app.restart();
}

/// Rejects the install flow early when the updater public key was never
/// replaced, so the user gets a clear message instead of an opaque signature
/// error deep inside the plugin.
fn ensure_updater_configured(app: &tauri::AppHandle) -> Result<(), String> {
    let pubkey = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|pubkey| pubkey.as_str())
        .unwrap_or("");

    if pubkey.trim().is_empty() || pubkey.trim() == UPDATER_PUBKEY_PLACEHOLDER {
        return Err(
            "Auto-update is not configured yet: replace the updater public key placeholder in \
             tauri.conf.json with a key generated by `cargo tauri signer generate`."
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(channel: AppBuildChannel, version: &str) -> AppBuildInfo {
        AppBuildInfo {
            version: version.to_string(),
            commit_sha: "12345678".to_string(),
            dirty: false,
            channel,
            display_version: "test".to_string(),
        }
    }

    fn release(tag: &str) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            html_url: "https://github.com/JustShinobi/NinjaCrawler/releases/tag/test".to_string(),
            published_at: Some("2026-07-13T00:00:00Z".to_string()),
        }
    }

    #[test]
    fn release_build_reports_newer_stable_version() {
        let status =
            status_from_release(build(AppBuildChannel::Release, "1.2.3"), release("v1.3.0"))
                .expect("status should parse");
        assert!(status.update_available);
        assert_eq!(status.latest_version, "1.3.0");
    }

    #[test]
    fn release_build_does_not_offer_equal_or_older_versions() {
        for tag in ["v1.2.3", "v1.2.2"] {
            let status =
                status_from_release(build(AppBuildChannel::Release, "1.2.3"), release(tag))
                    .expect("status should parse");
            assert!(!status.update_available, "{tag} must not be offered");
        }
    }

    #[test]
    fn development_build_never_claims_to_be_outdated() {
        let status = status_from_release(
            build(AppBuildChannel::Development, "0.1.0"),
            release("v99.0.0"),
        )
        .expect("status should parse");
        assert!(!status.update_available);
        assert_eq!(status.latest_version, "99.0.0");
    }

    #[test]
    fn strict_release_tag_rejects_prerelease_build_metadata_and_loose_formats() {
        for tag in ["1.2.3", "v1.2", "v1.2.3-beta.1", "v1.2.3+build", "v01.2.3"] {
            assert!(
                strict_release_version(tag).is_err(),
                "{tag} must be rejected"
            );
        }
    }

    #[test]
    fn unexpected_release_url_is_rejected() {
        let mut response = release("v1.3.0");
        response.html_url = "https://example.invalid/download".to_string();
        let error = status_from_release(build(AppBuildChannel::Release, "1.2.3"), response)
            .expect_err("unexpected URL must fail");
        assert!(error.contains("unexpected release URL"));
    }
}
