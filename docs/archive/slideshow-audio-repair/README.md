# TikTok slideshow audio repair (one-shot)

Archived for reference. **These files are not compiled** — they were moved out of
`src-tauri/src/` on 2026-07-20, once the repair had run its course.

TikTok slideshows are downloaded as images plus a separate soundtrack file. A stretch of
syncs saved the images but lost the audio, and this pass re-fetched it via yt-dlp, marking
posts unavailable when the source was gone instead of retrying them forever.

| file | role |
|---|---|
| `slideshow_audio_repair.rs` | scan, download and availability probing (was `workspace_repository/`) |
| `slideshow_audio_repair_cli.rs` | headless entry point (was `src/bin/`) |

## History worth knowing

It started as a panel in the Workspace Health window and was later reduced to a CLI, but
the UI half was never removed. When it was finally archived, five functions had been dead
for a while — `load_slideshow_audio_repair_panel`, `preview_slideshow_audio_repair`,
`dismiss_slideshow_audio_repair`, `clear_slideshow_audio_unavailable` and the
`AppHandle`-taking `run_slideshow_audio_repair` — along with five DTOs in `domain::models`.
None of it was reachable: no Tauri command, no frontend reference.

A subtler leftover: `preview_slideshow_audio_repair_full` was the *scan* that populated
`state.json`, which the download phase requires (`"No saved scan…"`). The CLI never exposed
it, so the tool only worked because an older `state.json` was still lying around from the
UI days. Anyone reviving this needs to expose the scan first.

## What stayed in the codebase

Nothing here is needed for TikTok slideshows to work normally. Still live, and unrelated
to the repair:

- `gallery::find_slideshow_audio` — pairs a slideshow with its soundtrack in the gallery;
- `tiktok_connector::persist_slideshow_audio` — saves the soundtrack during a normal sync;
- `health::open_app_log_file` / `reveal_app_log_file` — generic log helpers that happened
  to live in this module and moved to `health.rs`.
