# Media dedupe runtime

NinjaCrawler uses two independent engines for media cleanup:

- NinjaCrawler owns the persistent file catalog, SHA-256 exact matching, hardlink consolidation, review state, Recycle Bin actions, and audit log.
- [Video Duplicate Finder](https://github.com/0x90d/videoduplicatefinder) (VDF) is an optional external process used only to produce perceptual similarity candidates. VDF is licensed under AGPL-3.0.
- FFmpeg and FFprobe are shared media tools used by video similarity and NinjaCrawler video thumbnails. NinjaCrawler prefers a valid pair on the system `PATH`; otherwise it can install a private runtime without changing the system `PATH`.

The managed Windows runtime is downloaded from the upstream release, verified against the SHA-256 pinned in `media_dedupe_vdf.rs`, and extracted under the connector runtime directory. NinjaCrawler does not link to VDF libraries and never invokes its delete flags.

VDF releases are daily mutable assets. Installation fails closed if the upstream archive no longer matches the pinned digest. Updating the runtime requires reviewing the upstream commit, validating its CLI and JSON output, then updating the pinned version and digest together.

Each source has its own VDF database folder under `data/media-dedupe/vdf/<sourceId>`, with VDF writing `ScannedFiles.db` inside it. NinjaCrawler creates and validates the directory before launch and rejects a successful process that did not create the isolated database. This prevents VDF from falling back to the shared database beside its executable.

Perceptual matching remains source-scoped. Exact SHA-256 groups cross sources in the selected scan scope when the files are on the same volume. NinjaCrawler handles image similarity with its persisted aHash/dHash values; VDF receives videos only, avoiding duplicate image decoding. Source jobs with unchanged video inventories reuse their prior imported candidates when the runtime and settings fingerprints still match.

## Resource profiles

The operator chooses a resource profile when starting a scan:

- `Quiet` uses one VDF hashing and matching worker, one source lane, and lowers the child-process priority.
- `Balanced` is the default, allocates approximately half of the available logical processors, and allows up to two source lanes on different volumes.
- `Fast` reserves two logical processors for the desktop, gives the remaining budget to VDF at normal priority, and allows up to four source lanes on different volumes.

Sources on the same volume remain serial to avoid competing seeks and saturating a single disk. When multiple volumes are active, the shared CPU budget is divided among their concurrent VDF processes. Worker counts are computed at runtime, so the profiles scale with the host instead of encoding a machine-specific thread count. Progress reports the scan scope, source-scoped similarity semantics, elapsed time, rolling throughput, and an ETA once enough work has completed.

## FFmpeg distribution

The Windows installer uses the versioned GyanD essentials build linked by the official FFmpeg download page. The URL, version, and published SHA-256 are pinned in `media_tool_runtime.rs`. The complete verified archive is extracted under `connectors/media-tools/ffmpeg/<version>`, including its license and source information. The managed tools are passed only to NinjaCrawler child processes.

The long-term publication model should use a dedicated NinjaCrawler toolchain release rather than an application release:

1. Mirror the reviewed upstream archive without modifying it.
2. Publish it from a draft to a GitHub immutable release dedicated to media tools.
3. Generate provenance and SBOM attestations in the publishing workflow.
4. Keep a repository-owned signed manifest containing the upstream URL, immutable mirror URL, SHA-256, size, license, and supported platform.
5. Pin the desktop client to a manifest revision and continue verifying the archive digest locally.

An immutable release is preferable to a normal release asset because its tag and assets cannot be replaced after publication. Keeping media tools separate from app releases allows security updates without coupling them to the desktop version lifecycle. The direct versioned upstream URL remains suitable while the mirror pipeline is not yet published because installation fails closed on any digest mismatch.
