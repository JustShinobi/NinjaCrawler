# 4K Stogram → NinjaCrawler (one-shot migration)

Archived for reference. **These files are not compiled** — they were moved out of
`src-tauri/src/` after the migration ran, on 2026-07-20.

4K Stogram was discontinued. This code read its `.stogram.sqlite` catalog and moved a
68 GB library into the workspace: **168 profiles, 31.538 files, 48,22 GB, zero failures**,
in 16,7 minutes (39 profiles created, 129 merged into existing ones).

| file | role |
|---|---|
| `stogram_migration.rs` | the migration itself (was `workspace_repository/stogram_migration.rs`) |
| `stogram_migration_tests.rs` | its tests (11) |
| `stogram_migration_cli.rs` | headless entry point (was `src/bin/`) |
| `stogram_diag_cli.rs` | prints what the gallery returns for a profile; written to debug the album bug below |

## What stayed in the codebase

The migration needed a few things that outlive it and were kept:

- migration `0050_instagram_media_title_captured_at.sql` plus the `title`/`captured_at`
  columns on `instagram_sync_media_ledger` — the caption now shows in the lightbox;
- `reconcile_instagram_ledgers_from_records`, split out of the SCrawler import so any
  legacy catalog can seed the Instagram ledgers;
- `DownloadedInstagramMedia::file_sha256` / `image_fingerprint` and
  `InstagramFingerprintMedia::image_fingerprint`, which let a caller pass values it has
  already computed instead of the ledgers re-reading and re-decoding every file;
- `archived_profile_picture_path`, so an avatar archive can be dated by capture time
  rather than by "today".

## Things worth remembering

Findings that cost real debugging time, in case a similar import is ever written:

1. **`photos.is_video` is a bitfield, not a boolean.** Bit 0 flags video; the upper bits
   carry the section: 0/2/3 feed, 4/5 story, 16/17 highlight, 65 reel, 8 profile picture.
2. **Match profiles by user id, never by handle.** 19 of the 168 had changed their @ —
   matching by handle would have created duplicate profiles with the archive split in
   half, and in one case (`ttalia.xz`, an account recreated under the same @) it would
   have merged two identities. The 4K Stogram id never overwrites the workspace one:
   that catalog has been frozen since Feb/2025.
3. **Never hold the SQLite write lock during file I/O.** Copying (and, subtly, decoding
   images for the perceptual fingerprint) inside `BEGIN IMMEDIATE` blocked every other
   writer for minutes — the app itself would not start, dying on `SQLITE_BUSY` in its
   startup DDL with nothing in the Windows Event Log. The fix was three phases, with
   hashing/copying/fingerprinting outside any transaction.
4. **Always build with `--release` for bulk work.** In debug, `sha2` and `image` are
   ~250x slower: 1.099 files took 17,9 min; in release, 8.474 files plus 11,64 GB of
   copying took 5 min. The disks were never the bottleneck (870-895 MB/s).
5. **A highlight must live under an album directory.** The gallery reads the album from
   the *second* path segment (`stories/<album>/file`), so files dropped straight into
   `stories/` turn every file name into an album. Migrated highlights go to
   `stories/Legacy/`, or to the real album when it can be traced through the alias
   ledger. Derive the album from the real path, not the lowercased `relative_path`,
   or the casing difference creates a duplicate album.
6. **362 catalog rows are placeholders, not media**: all highlights, all flagged
   `state = 4` as if downloaded, averaging 4 KB against 2,6 MB for real ones. Detection
   requires two signals at once (CDN-style name *and* under 20 KB) — of the 4.570
   correctly named files, none is under 20 KB, so there are no false positives.
