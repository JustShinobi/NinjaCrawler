//! Headless one-shot slideshow-audio repair (no UI — this is the supported entry).
//!
//! Uses the live NinjaCrawler workspace under `%LOCALAPPDATA%\NinjaCrawler`
//! (DB, DPAPI session secrets, saved scan `state.json`, yt-dlp, media roots).
//!
//! Examples:
//! ```text
//! cargo run -p ninjacrawler --bin slideshow_audio_repair_cli -- --limit 20 --handle 2julinda
//! cargo run -p ninjacrawler --bin slideshow_audio_repair_cli -- --clear-unavailable --limit 50
//! cargo run -p ninjacrawler --bin slideshow_audio_repair_cli --
//! ```
//!
//! Prefer closing the NinjaCrawler UI while this writes `state.json` / the ledger.

use std::env;
use std::process;

use ninjacrawler_lib::infrastructure::workspace_repository::{
    run_slideshow_audio_repair_with_options, SlideshowAudioRepairRunOptions,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("slideshow_audio_repair_cli failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = parse_args(env::args().skip(1))?;
    eprintln!(
        "slideshow_audio_repair_cli: limit={:?} handle={:?} clear_unavailable={}",
        options.limit, options.handle_filter, options.clear_unavailable
    );
    eprintln!("Using live workspace (%LOCALAPPDATA%\\NinjaCrawler). Prefer UI closed.");

    let result = run_slideshow_audio_repair_with_options(None, options)?;

    println!("--- result ---");
    println!("attempted={}", result.attempted);
    println!("recovered={}", result.recovered);
    println!("failed(marked)={}", result.failed);
    println!("skipped={}", result.skipped);
    println!("marked_unavailable={}", result.marked_unavailable);
    println!("remaining_missing={}", result.remaining_missing);
    println!("requeued_transient={}", result.requeued_transient);
    println!("aborted_on_network_block={}", result.aborted_on_network_block);
    if let Some(path) = result.log_path.as_ref() {
        println!("log_path={path}");
    }
    if !result.failures.is_empty() {
        println!("failure_samples={}:", result.failures.len());
        for line in result.failures.iter().take(15) {
            println!("  - {line}");
        }
    }
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<SlideshowAudioRepairRunOptions, String> {
    let mut options = SlideshowAudioRepairRunOptions::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--clear-unavailable" => options.clear_unavailable = true,
            "--limit" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--limit requires a number".to_string())?;
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("invalid --limit {value}"))?;
                options.limit = Some(n);
            }
            "--handle" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--handle requires a name".to_string())?;
                options.handle_filter = Some(value);
            }
            other if other.starts_with("--limit=") => {
                let value = &other["--limit=".len()..];
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("invalid --limit {value}"))?;
                options.limit = Some(n);
            }
            other if other.starts_with("--handle=") => {
                options.handle_filter = Some(other["--handle=".len()..].to_string());
            }
            other => return Err(format!("unknown argument: {other} (try --help)")),
        }
    }
    Ok(options)
}

fn print_help() {
    eprintln!(
        "\
slideshow_audio_repair_cli — headless TikTok slideshow soundtrack repair

Usage:
  slideshow_audio_repair_cli [options]

Options:
  --limit N              Process at most N jobs
  --handle NAME          Only this TikTok handle (with or without @)
  --clear-unavailable    Clear the inaccessible ledger before running
  -h, --help             Show this help

Notes:
  • Requires a saved scan (cache/slideshow-audio-repair/state.json).
    If missing, run once with a scan from an older build, or call the Rust
    preview API / re-add a scan helper later.
  • Writes logs under logs/slideshow-audio-repair-*.log.
  • Single Videos downloads slideshow IMAGES; this tool repairs AUDIO only.
    A post can be \"downloadable\" as photos and still have no extractable
    soundtrack (yt-dlp: no formats / empty music.playUrl) — those stay queued.
"
    );
}
