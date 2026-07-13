use std::time::Duration;

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;

use crate::domain::models::{AppBuildChannel, AppBuildInfo, AppUpdateStatus};

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/MetalDevOps/NinjaCrawler/releases/latest";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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
    if !release
        .html_url
        .starts_with("https://github.com/MetalDevOps/NinjaCrawler/releases/")
    {
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
            html_url: "https://github.com/MetalDevOps/NinjaCrawler/releases/tag/test".to_string(),
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
