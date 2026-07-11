# Changelog

## [0.14.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.13.0...v0.14.0) (2026-07-11)


### Features

* **companion:** add keyboard shortcuts, themes, and update guidance ([#99](https://github.com/MetalDevOps/NinjaCrawler/issues/99)) ([0cb7300](https://github.com/MetalDevOps/NinjaCrawler/commit/0cb7300299a4d8fc0ad2b29de4c6679e3d047577))
* **release:** version the Companion on an independent track ([#103](https://github.com/MetalDevOps/NinjaCrawler/issues/103)) ([4a178db](https://github.com/MetalDevOps/NinjaCrawler/commit/4a178db42a38bd6b52a6abcf80071b6378b190c0))


### Bug Fixes

* **companion:** keep manifest mergeable into main ([d7990a2](https://github.com/MetalDevOps/NinjaCrawler/commit/d7990a22a56e7326f787ddf5004fc03ab8538d07))

## [0.13.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.12.0...v0.13.0) (2026-07-09)


### Features

* **instagram:** add incremental feed discovery and full-scan override ([#91](https://github.com/MetalDevOps/NinjaCrawler/issues/91)) ([000052c](https://github.com/MetalDevOps/NinjaCrawler/commit/000052ccc8c4461f4e34b5e500d2cd9cd6805d7b))


### Bug Fixes

* **tiktok:** preserve sync history on transient empty provider listings ([#92](https://github.com/MetalDevOps/NinjaCrawler/issues/92)) ([f3bff77](https://github.com/MetalDevOps/NinjaCrawler/commit/f3bff770b25ec8f3ec982ef934137acfc24f70d9))

## [0.12.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.11.0...v0.12.0) (2026-07-08)


### Features

* **profile-view:** improve multi-window player UX ([#87](https://github.com/MetalDevOps/NinjaCrawler/issues/87)) ([ec48a86](https://github.com/MetalDevOps/NinjaCrawler/commit/ec48a861b2fc6c38b40cf5479e37bda8b587099a))
* **single-videos:** support TikTok photo slideshow downloads ([#86](https://github.com/MetalDevOps/NinjaCrawler/issues/86)) ([b7445be](https://github.com/MetalDevOps/NinjaCrawler/commit/b7445bedf45c9fe76b567b0ec2f1d81c408f1535))

## [0.11.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.10.0...v0.11.0) (2026-07-07)


### Features

* **companion:** add profile sync action ([#76](https://github.com/MetalDevOps/NinjaCrawler/issues/76)) ([01d3ea9](https://github.com/MetalDevOps/NinjaCrawler/commit/01d3ea99c9da16d21d6b2663493c4ecabf132387))
* detect and flag unavailable/private profiles (TikTok + Instagram) ([#73](https://github.com/MetalDevOps/NinjaCrawler/issues/73)) ([e638398](https://github.com/MetalDevOps/NinjaCrawler/commit/e638398baf96731bfc5da75d423163ac114e89c8))
* **instagram:** categorize legacy SCrawler reels correctly ([#72](https://github.com/MetalDevOps/NinjaCrawler/issues/72)) ([9e25d6c](https://github.com/MetalDevOps/NinjaCrawler/commit/9e25d6c1100fe00791c512246eb0c12aebf8bd91))

## [0.10.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.9.0...v0.10.0) (2026-07-06)


### Features

* harden webview CSP and close UI↔backend gaps ([#66](https://github.com/MetalDevOps/NinjaCrawler/issues/66)) ([824620e](https://github.com/MetalDevOps/NinjaCrawler/commit/824620efb41d6a437e8a930d19f5157b4fbf5e08))
* **tiktok:** collect, persist, and sort media stats ([#64](https://github.com/MetalDevOps/NinjaCrawler/issues/64)) ([8ad2217](https://github.com/MetalDevOps/NinjaCrawler/commit/8ad2217e8f75c973020e246f38a3d68d2f0c3bab))
* **workspace:** implement media thumbnails, profile-view virtualization, and zero-distinction mono fonts ([#65](https://github.com/MetalDevOps/NinjaCrawler/issues/65)) ([d7e689b](https://github.com/MetalDevOps/NinjaCrawler/commit/d7e689b67fc599146ab05b323fae1d493023b3fe))


### Bug Fixes

* **instagram:** POST GraphQL queries so reels and feed sync ([#62](https://github.com/MetalDevOps/NinjaCrawler/issues/62)) ([be45445](https://github.com/MetalDevOps/NinjaCrawler/commit/be4544555489b451691abf4f3c7bae37e956a900))

## [0.9.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.8.1...v0.9.0) (2026-07-04)


### Features

* **tiktok:** add authenticated liked-video sync ([03ea0a8](https://github.com/MetalDevOps/NinjaCrawler/commit/03ea0a8b3e779a16c3c70c68d4399ed8f688bd90))
* **tiktok:** add liked videos sync support ([c473194](https://github.com/MetalDevOps/NinjaCrawler/commit/c473194d17a05413d29df21938890149be3b1d78))
* **workspace:** improve sync panel tooltips ([c7ec9f7](https://github.com/MetalDevOps/NinjaCrawler/commit/c7ec9f7c649dbc480f5f26d65e72477aa86a8d37))

## [0.8.1](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.8.0...v0.8.1) (2026-07-04)


### Bug Fixes

* **workspace:** prevent profile context menu from overflowing window ([4330085](https://github.com/MetalDevOps/NinjaCrawler/commit/433008594b8a94bf573b34ecafc7a7984f74de89))
* **workspace:** prevent profile context menu from overflowing window ([34a8296](https://github.com/MetalDevOps/NinjaCrawler/commit/34a8296e294e11d84a4a69a8f8fe704e28f1246a))

## [0.8.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.7.0...v0.8.0) (2026-07-04)


### Features

* **companion:** add batch companion flows and release packaging support ([9aec1c0](https://github.com/MetalDevOps/NinjaCrawler/commit/9aec1c0ac3707aa030409d1a364a941a3982082a))

## [0.7.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.6.0...v0.7.0) (2026-07-03)


### Features

* **workspace:** flag profiles with stale last-sync ([a0990af](https://github.com/MetalDevOps/NinjaCrawler/commit/a0990af0d9050d20a9e279038d9f03c974e16f2d))
* **workspace:** flag profiles with stale last-sync ([5b38ffe](https://github.com/MetalDevOps/NinjaCrawler/commit/5b38ffec5540ec42a6482717a17860edead94758))


### Bug Fixes

* **instagram:** reconcile imported identity hints ([5fe9b32](https://github.com/MetalDevOps/NinjaCrawler/commit/5fe9b329b29c8a398a23d0d9cb61603b75192ce3))
* **instagram:** reconcile imported user ID hints with confirmed history ([afc4123](https://github.com/MetalDevOps/NinjaCrawler/commit/afc4123957a9749b741ea5127e2223be75f9b600))


### Code Refactoring

* **companion:** remove Reddit detection and gating ([d17e3a6](https://github.com/MetalDevOps/NinjaCrawler/commit/d17e3a6552b618dc8e97b5366b89993d5e8a5bfd))
* drop the Reddit provider from the app core ([41be930](https://github.com/MetalDevOps/NinjaCrawler/commit/41be9309f91f34561332644f0f2cb32ae28724aa))
