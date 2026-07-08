# Companion Release Packaging

## Understanding summary

- GitHub releases provide the NinjaCrawler Companion as a ZIP for manual installation.
- The Companion version is declared in its `manifest.json` and is included in
  the automated Release Please version bump.
- The archive contains only runtime files and user documentation.
- The existing release workflow publishes the archive and its SHA-256 checksum.

## Design

`Tools/Package-NinjaCrawlerRelease.ps1` validates the Companion manifest and
required files, stages the distributable contents, and creates
`NinjaCrawler-Companion-<version>.zip`. The archive has a stable
`NinjaCrawler-Companion` root folder so a new release can be extracted over the
existing unpacked extension directory.

JavaScript test files are excluded. The archive is added to the same asset list
used for `SHA256SUMS.txt`; the release workflow already uploads every file from
the output directory.

Release Please updates `NinjaCrawler.Companion/manifest.json` in the release PR
alongside the desktop version files. This keeps Companion-only changes from
shipping in a new GitHub release with the previous extension version.

The packaging script supports a Companion-only mode so CI can verify the archive
layout and checksum without compiling the desktop application.

## Decision log

1. **ZIP distribution.** Off-store CRX signing and browser-store publication are
   outside the current scope.
2. **Stable unpacked identity.** The manifest includes a fixed public key and
   the ZIP uses a stable root folder. Together they prevent each extracted
   release from becoming a second Chrome extension.
3. **Manifest-driven version.** The asset name uses the manifest version read at
   packaging time.
4. **Shared packager.** Desktop and Companion assets use one release script and
   one checksum file.
5. **Runtime-only contents.** Extension tests remain in source control but are
   omitted from published archives.
