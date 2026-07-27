//! Headless one-shot migration of a 4K Stogram library into NinjaCrawler.
//!
//! 4K Stogram was discontinued; this reads its `.stogram.sqlite` catalog, copies
//! the media into `<media_root>/instagram/<handle>` and seeds the Instagram
//! ledgers. The 4K Stogram library itself is only ever read.
//!
//! Writes to the live workspace under `%LOCALAPPDATA%\NinjaCrawler` — close the
//! NinjaCrawler UI and back up `data\ninjacrawler.db` before a real run.
//!
//! Examples:
//! ```text
//! cargo run -p ninjacrawler --bin stogram_migration_cli -- --root "D:\4K Stogram" --account lisalalisaa2 --dry-run
//! cargo run -p ninjacrawler --bin stogram_migration_cli -- --root "D:\4K Stogram" --account lisalalisaa2 --handle 0julinda
//! cargo run -p ninjacrawler --bin stogram_migration_cli -- --root "D:\4K Stogram" --account lisalalisaa2
//! ```

use std::env;
use std::path::PathBuf;
use std::process;

use ninjacrawler_lib::infrastructure::workspace_repository::{
    run_stogram_migration, StogramMigrationOptions,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("stogram_migration_cli failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = parse_args(env::args().skip(1))?;
    eprintln!(
        "stogram_migration_cli: root={} account={:?} handles={:?} limit={:?} dry_run={}",
        options.stogram_root.display(),
        options.account,
        options.handles,
        options.limit,
        options.dry_run
    );
    if options.dry_run {
        eprintln!("Dry run: nothing is copied and nothing is written to the workspace.");
    } else {
        eprintln!("Writing to the live workspace (%LOCALAPPDATA%\\NinjaCrawler). Prefer UI closed.");
    }

    let report = run_stogram_migration(options, &mut |outcome| {
        eprintln!(
            "  [{}] {} -> {} ({}): {}",
            outcome.status,
            outcome.stogram_handle,
            outcome.source_handle.as_deref().unwrap_or("-"),
            outcome.matched_by,
            outcome.message
        );
    })?;

    println!("--- result ---");
    println!("dry_run={}", report.dry_run);
    println!("profiles_total={}", report.profiles_total);
    println!("profiles_created={}", report.profiles_created);
    println!("profiles_merged={}", report.profiles_merged);
    println!("profiles_failed={}", report.profiles_failed);
    println!("media_copied={}", report.media_copied);
    println!("media_already_cataloged={}", report.media_already_cataloged);
    println!("media_recovered_from_disk={}", report.media_recovered);
    println!(
        "media_skipped_degraded={} (thumbnail-sized placeholders in the 4K Stogram library)",
        report.media_skipped_degraded
    );
    println!("media_missing_files={}", report.media_missing_files);
    println!("avatars_promoted={}", report.avatars_promoted);
    println!("avatars_archived={}", report.avatars_archived);
    println!(
        "highlight_albums_matched={} (rest go to the 'Legacy' album)",
        report.highlight_albums_matched
    );
    println!(
        "gb_copied={:.2}",
        report.bytes_copied as f64 / (1024.0 * 1024.0 * 1024.0)
    );

    // Merges resolved by handle alone are the only ones not confirmed by user
    // id, so they are worth eyeballing after the run.
    let handle_merges = report
        .profiles
        .iter()
        .filter(|outcome| outcome.matched_by == "handle")
        .collect::<Vec<_>>();
    if !handle_merges.is_empty() {
        println!("handle_matched_merges={}:", handle_merges.len());
        for outcome in handle_merges {
            println!(
                "  - {} (stogram id {}) -> {}",
                outcome.stogram_handle,
                outcome.user_id,
                outcome.source_handle.as_deref().unwrap_or("-")
            );
        }
    }

    let failures = report
        .profiles
        .iter()
        .filter(|outcome| outcome.status == "failed")
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        println!("failures={}:", failures.len());
        for outcome in failures.iter().take(15) {
            println!("  - {}: {}", outcome.stogram_handle, outcome.message);
        }
    }

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<StogramMigrationOptions, String> {
    let mut stogram_root = None;
    let mut account = None;
    let mut handles = Vec::new();
    let mut limit = None;
    let mut dry_run = false;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                stogram_root = Some(PathBuf::from(
                    args.next().ok_or_else(|| "--root needs a path.".to_string())?,
                ));
            }
            "--account" => {
                account = Some(
                    args.next()
                        .ok_or_else(|| "--account needs an id or display name.".to_string())?,
                );
            }
            "--handle" => {
                handles.push(
                    args.next()
                        .ok_or_else(|| "--handle needs a value.".to_string())?,
                );
            }
            "--limit" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--limit needs a number.".to_string())?;
                limit = Some(
                    raw.parse::<usize>()
                        .map_err(|error| format!("Invalid --limit '{raw}': {error}"))?,
                );
            }
            "--dry-run" => dry_run = true,
            other => return Err(format!("Unknown argument '{other}'.")),
        }
    }

    Ok(StogramMigrationOptions {
        stogram_root: stogram_root
            .ok_or_else(|| "--root is required (the 4K Stogram library folder).".to_string())?,
        account,
        handles,
        limit,
        dry_run,
    })
}
