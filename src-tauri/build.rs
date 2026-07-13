use std::{env, process::Command};

const UNKNOWN_SHA: &str = "unknown";

fn git_output(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn short_sha(value: &str) -> Option<String> {
    let normalized = value.trim();
    if normalized.len() < 8
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }

    Some(normalized[..8].to_ascii_lowercase())
}

fn main() {
    for name in [
        "NINJACRAWLER_RELEASE_BUILD",
        "NINJACRAWLER_RELEASE_VERSION",
        "NINJACRAWLER_BUILD_SHA",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    for path in [
        "build.rs",
        "src",
        "../src",
        "../package.json",
        "../package-lock.json",
        "tauri.conf.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for git_path in ["HEAD", "index"] {
        if let Some(path) = git_output(&["rev-parse", "--git-path", git_path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git_output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    let release_build = matches!(
        env::var("NINJACRAWLER_RELEASE_BUILD").as_deref(),
        Ok("1") | Ok("true")
    );
    let package_version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is required");

    let (channel, sha, dirty) = if release_build {
        let release_version = env::var("NINJACRAWLER_RELEASE_VERSION")
            .expect("NINJACRAWLER_RELEASE_VERSION is required for official release builds");
        if release_version != package_version {
            panic!(
                "official release version '{release_version}' does not match Cargo package version '{package_version}'"
            );
        }

        let sha = env::var("NINJACRAWLER_BUILD_SHA")
            .ok()
            .and_then(|value| short_sha(&value))
            .expect(
                "NINJACRAWLER_BUILD_SHA must contain a Git commit SHA for official release builds",
            );
        ("release", sha, false)
    } else {
        let sha = env::var("NINJACRAWLER_BUILD_SHA")
            .ok()
            .and_then(|value| short_sha(&value))
            .or_else(|| {
                git_output(&["rev-parse", "--short=8", "HEAD"]).and_then(|value| short_sha(&value))
            })
            .unwrap_or_else(|| UNKNOWN_SHA.to_string());
        let dirty = git_output(&["status", "--porcelain", "--untracked-files=normal"])
            .is_some_and(|value| !value.trim().is_empty());
        ("development", sha, dirty)
    };

    println!("cargo:rustc-env=NINJACRAWLER_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=NINJACRAWLER_BUILD_SHA={sha}");
    println!(
        "cargo:rustc-env=NINJACRAWLER_BUILD_DIRTY={}",
        if dirty { "true" } else { "false" }
    );

    tauri_build::build()
}
