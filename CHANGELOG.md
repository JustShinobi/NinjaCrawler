# Changelog

## [0.24.0](https://github.com/JustShinobi/NinjaCrawler/compare/v0.23.0...v0.24.0) (2026-07-16)


### Features

* **companion:** add batch companion flows and release packaging support ([9aec1c0](https://github.com/JustShinobi/NinjaCrawler/commit/9aec1c0ac3707aa030409d1a364a941a3982082a))
* **companion:** add keyboard shortcuts, themes, and update guidance ([#99](https://github.com/JustShinobi/NinjaCrawler/issues/99)) ([0cb7300](https://github.com/JustShinobi/NinjaCrawler/commit/0cb7300299a4d8fc0ad2b29de4c6679e3d047577))
* **companion:** capture single videos by URL ([6f481ad](https://github.com/JustShinobi/NinjaCrawler/commit/6f481addd540911d547202bc123cda007843ba6c))
* **companion:** capture TikTok stories into the profile's Stories folder ([2abb56e](https://github.com/JustShinobi/NinjaCrawler/commit/2abb56e61f89dc8614efd4bd3af828f9b241c9bd))
* **companion:** download selected Instagram stories ([9e33a29](https://github.com/JustShinobi/NinjaCrawler/commit/9e33a298017f67fb666cc880b290a2963f34f3e1))
* **companion:** harden account capture and block incomplete session imports ([#102](https://github.com/JustShinobi/NinjaCrawler/issues/102)) ([5e53316](https://github.com/JustShinobi/NinjaCrawler/commit/5e53316b94b2ea1191add5f1a551a5924987af94))
* **companion:** import browser account sessions ([5ba11cf](https://github.com/JustShinobi/NinjaCrawler/commit/5ba11cf82366432f27706a974ec1f0e4b0a7f595))
* **companion:** import browser account sessions ([e098415](https://github.com/JustShinobi/NinjaCrawler/commit/e0984153f1e0bed55f6528751fa5b25dd61f8c0b))
* **companion:** stage updates in AppData and detect first Instagram story ([#22](https://github.com/JustShinobi/NinjaCrawler/issues/22)) ([ad7577c](https://github.com/JustShinobi/NinjaCrawler/commit/ad7577c4a311a608fdebc182d87f83bcfa8314d9))
* **connectors:** add managed connector bootstrap ([b5e60ba](https://github.com/JustShinobi/NinjaCrawler/commit/b5e60ba3a016fdc1456328e63ed9fa53c78a2c0b))
* **connectors:** implement Twitter incremental sync and rate limit holds ([#112](https://github.com/JustShinobi/NinjaCrawler/issues/112)) ([3b8992c](https://github.com/JustShinobi/NinjaCrawler/commit/3b8992c11e28fac153afd247616bb571e46b76ee))
* **core:** add the Tauri desktop runtime ([57b7c14](https://github.com/JustShinobi/NinjaCrawler/commit/57b7c144f105ea52aebf566830559c5df62a6a68))
* **debugger:** Add realtime connector diagnostics ([a265b81](https://github.com/JustShinobi/NinjaCrawler/commit/a265b81f004d583c6188e6c07cff12566f8c01d9))
* **debugger:** Add realtime connector diagnostics ([8edc610](https://github.com/JustShinobi/NinjaCrawler/commit/8edc610e0b66ff97f061ed97ddc40fb0f5a6d148))
* detect and flag unavailable/private profiles (TikTok + Instagram) ([#73](https://github.com/JustShinobi/NinjaCrawler/issues/73)) ([e638398](https://github.com/JustShinobi/NinjaCrawler/commit/e638398baf96731bfc5da75d423163ac114e89c8))
* harden webview CSP and close UI↔backend gaps ([#66](https://github.com/JustShinobi/NinjaCrawler/issues/66)) ([824620e](https://github.com/JustShinobi/NinjaCrawler/commit/824620efb41d6a437e8a930d19f5157b4fbf5e08))
* **instagram:** add incremental feed discovery and full-scan override ([#91](https://github.com/JustShinobi/NinjaCrawler/issues/91)) ([000052c](https://github.com/JustShinobi/NinjaCrawler/commit/000052ccc8c4461f4e34b5e500d2cd9cd6805d7b))
* **instagram:** categorize legacy SCrawler reels correctly ([#72](https://github.com/JustShinobi/NinjaCrawler/issues/72)) ([9e25d6c](https://github.com/JustShinobi/NinjaCrawler/commit/9e25d6c1100fe00791c512246eb0c12aebf8bd91))
* **instagram:** support targeted story downloads ([54a8199](https://github.com/JustShinobi/NinjaCrawler/commit/54a819990044256012225601f8ceb6d033c4b503))
* **profile-view:** add view-mode toggle and thumbnail density control ([323d65a](https://github.com/JustShinobi/NinjaCrawler/commit/323d65a21cf3fcfd862bd1106d293e9e92653943))
* **profile-view:** delete media (single + multi-select) with re-download tombstone ([e2c19db](https://github.com/JustShinobi/NinjaCrawler/commit/e2c19db1ce0b5b829ac2bc68ff3dfabcbeebebaa))
* **profile-view:** delete media (single + multi-select) with re-download tombstone ([8834c73](https://github.com/JustShinobi/NinjaCrawler/commit/8834c73da7b1bef2419422f297ac092311453a13))
* **profile-view:** group highlights by album and fix their downloads ([1773c6e](https://github.com/JustShinobi/NinjaCrawler/commit/1773c6e6e86f57a7c4829f25df7da3f4505db1b9))
* **profile-view:** group Instagram carousels by shortcode ([3393ec4](https://github.com/JustShinobi/NinjaCrawler/commit/3393ec408eac0feaa99f0966924b2591bb993b06))
* **profile-view:** group Instagram carousels by shortcode ([28083b5](https://github.com/JustShinobi/NinjaCrawler/commit/28083b5f4e374ae855b6a7d3e2e4a825f61cbda7))
* **profile-view:** hide the Online link for stories ([1805b08](https://github.com/JustShinobi/NinjaCrawler/commit/1805b08ddc8b00869dc849a3e1f7c70b05ec13f9))
* **profile-view:** hide the Online link for stories ([f23763e](https://github.com/JustShinobi/NinjaCrawler/commit/f23763e2d314be31a985318e5511c78cc16154e8))
* **profile-view:** highlights grouped by album + fix highlight downloads (v0.4.0) ([65e53b1](https://github.com/JustShinobi/NinjaCrawler/commit/65e53b19735727caef27a757e620e3e2ae9ad744))
* **profile-view:** improve multi-window player UX ([#87](https://github.com/JustShinobi/NinjaCrawler/issues/87)) ([ec48a86](https://github.com/JustShinobi/NinjaCrawler/commit/ec48a861b2fc6c38b40cf5479e37bda8b587099a))
* **profile-view:** media deletion UX rework, pinned header, release v0.3.0 ([defb9f0](https://github.com/JustShinobi/NinjaCrawler/commit/defb9f08801f52ef06634b3748699b8703c9debd))
* **profile-view:** rebuild Instagram/Twitter post links and split feed vs reels ([2304c16](https://github.com/JustShinobi/NinjaCrawler/commit/2304c16d37e9e9a31aae1235931e1162fdcc8c8e))
* **profile-view:** rebuild Instagram/Twitter post links and split feed vs reels ([b16db90](https://github.com/JustShinobi/NinjaCrawler/commit/b16db90fa801a281c86c76faa142c9527a8298e6))
* **profile-view:** rework media deletion UX and pin the header ([140d582](https://github.com/JustShinobi/NinjaCrawler/commit/140d582c5011d26975e4e4dc7cddd63742b9c640))
* **profile-view:** view-mode toggle and thumbnail density control ([d694182](https://github.com/JustShinobi/NinjaCrawler/commit/d69418229fdbd11c3245e1a56d659ce1483fbdf6))
* **queue:** add media path migration cancellation and detailed progress ([#123](https://github.com/JustShinobi/NinjaCrawler/issues/123)) ([95b97b7](https://github.com/JustShinobi/NinjaCrawler/commit/95b97b75def12c4f1f27d53193899e925c897dfd))
* **release:** add app update checker and release back-sync ([#140](https://github.com/JustShinobi/NinjaCrawler/issues/140)) ([58458d5](https://github.com/JustShinobi/NinjaCrawler/commit/58458d5bfad08a95aede5f2bda377833969af000))
* **release:** cross-build thin Windows distribution ([#160](https://github.com/JustShinobi/NinjaCrawler/issues/160)) ([408bb96](https://github.com/JustShinobi/NinjaCrawler/commit/408bb96b820f6a6015f46dea4590845ff7692ffa))
* **release:** Publish Companion archive ([5d75abe](https://github.com/JustShinobi/NinjaCrawler/commit/5d75abea54f468eb9bdb126b976c33400f32cd7e))
* **release:** version the Companion on an independent track ([#103](https://github.com/JustShinobi/NinjaCrawler/issues/103)) ([4a178db](https://github.com/JustShinobi/NinjaCrawler/commit/4a178db42a38bd6b52a6abcf80071b6378b190c0))
* Single Videos (queue, gallery, add-profile) + TikTok story capture ([b17edca](https://github.com/JustShinobi/NinjaCrawler/commit/b17edca28dcc8796fe80f4a0701a9451b91eaefa))
* **single-videos:** add the Single Videos window (add-by-URL + filters) ([cd4c50b](https://github.com/JustShinobi/NinjaCrawler/commit/cd4c50ba7a12ce48d7909caf1386dfbe99bdb10a))
* **single-videos:** data model + provider-agnostic yt-dlp downloader ([4500702](https://github.com/JustShinobi/NinjaCrawler/commit/4500702544f765b32c35d737129e8da3d633ca16))
* **single-videos:** gallery parity, live refresh and add-profile menu ([e582f8f](https://github.com/JustShinobi/NinjaCrawler/commit/e582f8f30db9ebed513315e1dc295325fe020aa0))
* **single-videos:** queue downloads via a dedicated runtime ([35a9532](https://github.com/JustShinobi/NinjaCrawler/commit/35a9532e91d605600b059c1ff5f87d7bf7b11062))
* **single-videos:** support TikTok photo slideshow downloads ([#86](https://github.com/JustShinobi/NinjaCrawler/issues/86)) ([b7445be](https://github.com/JustShinobi/NinjaCrawler/commit/b7445bedf45c9fe76b567b0ec2f1d81c408f1535))
* **source-editor:** allow manually editing a locked handle ([d4927c9](https://github.com/JustShinobi/NinjaCrawler/commit/d4927c91e22ea8ad871ce385c7abd187a97954f2))
* **source-editor:** manual handle edit + history record (v0.5.1) ([7fac4e8](https://github.com/JustShinobi/NinjaCrawler/commit/7fac4e8bddebb6d96e08bf12d8348d7e031bf205))
* **source-editor:** record manual handle changes in profile history ([b072187](https://github.com/JustShinobi/NinjaCrawler/commit/b0721870193f492fdb9fa5382321ba29592928c2))
* **tiktok:** add authenticated liked-video sync ([03ea0a8](https://github.com/JustShinobi/NinjaCrawler/commit/03ea0a8b3e779a16c3c70c68d4399ed8f688bd90))
* **tiktok:** add liked videos sync support ([c473194](https://github.com/JustShinobi/NinjaCrawler/commit/c473194d17a05413d29df21938890149be3b1d78))
* **tiktok:** collect, persist, and sort media stats ([#64](https://github.com/JustShinobi/NinjaCrawler/issues/64)) ([8ad2217](https://github.com/JustShinobi/NinjaCrawler/commit/8ad2217e8f75c973020e246f38a3d68d2f0c3bab))
* **twitter:** backfill post key on existing media + stop bogus filename links ([2761a68](https://github.com/JustShinobi/NinjaCrawler/commit/2761a6849b26d8c84d395fc1be61f70da81a3b1d))
* **twitter:** backfill post key on existing media + stop bogus filename links ([6fde66f](https://github.com/JustShinobi/NinjaCrawler/commit/6fde66f9ca6339fc98b8f889d681e3eb6b0c49b2))
* **twitter:** warn when account cannot view sensitive media ([#43](https://github.com/JustShinobi/NinjaCrawler/issues/43)) ([a3731e3](https://github.com/JustShinobi/NinjaCrawler/commit/a3731e3338559d51b71d0c446376ba857ac7d6eb))
* **ui:** add the desktop workspace interface ([ca917a4](https://github.com/JustShinobi/NinjaCrawler/commit/ca917a4fcbe093fb494c446dbfff77ac08913224))
* **windows:** migrate remaining Tauri windows to WindowShell ([#11](https://github.com/JustShinobi/NinjaCrawler/issues/11)) ([87d5155](https://github.com/JustShinobi/NinjaCrawler/commit/87d5155e6dcbb4ed70edd85ead45f36a108fbf32))
* **windows:** migrate utility windows to custom visual shell ([#2](https://github.com/JustShinobi/NinjaCrawler/issues/2)) ([6dd8c63](https://github.com/JustShinobi/NinjaCrawler/commit/6dd8c63c6f82bb26de7718362b12685b1f67c6b0))
* **workspace:** establish brand visual system and custom window shell ([#193](https://github.com/JustShinobi/NinjaCrawler/issues/193)) ([1d91f7f](https://github.com/JustShinobi/NinjaCrawler/commit/1d91f7feaea5090a1d22779f5b75a7eb85a8723a))
* **workspace:** flag profiles with stale last-sync ([a0990af](https://github.com/JustShinobi/NinjaCrawler/commit/a0990af0d9050d20a9e279038d9f03c974e16f2d))
* **workspace:** flag profiles with stale last-sync ([5b38ffe](https://github.com/JustShinobi/NinjaCrawler/commit/5b38ffec5540ec42a6482717a17860edead94758))
* **workspace:** implement media thumbnails, profile-view virtualization, and zero-distinction mono fonts ([#65](https://github.com/JustShinobi/NinjaCrawler/issues/65)) ([d7e689b](https://github.com/JustShinobi/NinjaCrawler/commit/d7e689b67fc599146ab05b323fae1d493023b3fe))
* **workspace:** improve sync panel tooltips ([c7ec9f7](https://github.com/JustShinobi/NinjaCrawler/commit/c7ec9f7c649dbc480f5f26d65e72477aa86a8d37))
* **workspace:** process media path migrations in a background queue ([#119](https://github.com/JustShinobi/NinjaCrawler/issues/119)) ([07cc3a2](https://github.com/JustShinobi/NinjaCrawler/commit/07cc3a2ee5a27536c09b4a8f68f9c8090cef640e))


### Bug Fixes

* **build:** resolve connector release download URLs ([4324f4c](https://github.com/JustShinobi/NinjaCrawler/commit/4324f4c5dafe34f495b3a71148b4b0520c7a4fb3))
* **instagram:** POST GraphQL queries so reels and feed sync ([#62](https://github.com/JustShinobi/NinjaCrawler/issues/62)) ([be45445](https://github.com/JustShinobi/NinjaCrawler/commit/be4544555489b451691abf4f3c7bae37e956a900))
* **instagram:** reconcile imported identity hints ([5fe9b32](https://github.com/JustShinobi/NinjaCrawler/commit/5fe9b329b29c8a398a23d0d9cb61603b75192ce3))
* **instagram:** reconcile imported user ID hints with confirmed history ([afc4123](https://github.com/JustShinobi/NinjaCrawler/commit/afc4123957a9749b741ea5127e2223be75f9b600))
* **instagram:** restore profile note sync ([#212](https://github.com/JustShinobi/NinjaCrawler/issues/212)) ([106b200](https://github.com/JustShinobi/NinjaCrawler/commit/106b20082febbc9bd04a8840325293686c07e08b))
* **profile-view:** rebuild Twitter post links from the legacy SCrawler XML ([05029a2](https://github.com/JustShinobi/NinjaCrawler/commit/05029a2e124ae221eaf1e0c0211785297176aa8a))
* **profiles:** recover renamed identities across providers ([8d8c269](https://github.com/JustShinobi/NinjaCrawler/commit/8d8c269576963ebd5fe39712b58cbfbd276d8a5e))
* **profiles:** Recover renamed provider identities ([d90c756](https://github.com/JustShinobi/NinjaCrawler/commit/d90c756136d1f9f3fa4d1fe1b107c696066a29fe))
* **profiles:** Stabilize saved profile selection ([4f52d80](https://github.com/JustShinobi/NinjaCrawler/commit/4f52d803f8594b99ca29a8cadd1b6ae69310394d))
* **queue:** dispatch image thumbnail generation correctly ([#127](https://github.com/JustShinobi/NinjaCrawler/issues/127)) ([44ed4e3](https://github.com/JustShinobi/NinjaCrawler/commit/44ed4e3977eaf5323094cac935bc863b752bef57))
* **release:** authenticate recovery with repository PAT ([#130](https://github.com/JustShinobi/NinjaCrawler/issues/130)) ([be6eed0](https://github.com/JustShinobi/NinjaCrawler/commit/be6eed0618427875cd42053fad3e03a3c6a15b06))
* **release:** handle missing draft releases safely ([#206](https://github.com/JustShinobi/NinjaCrawler/issues/206)) ([2258472](https://github.com/JustShinobi/NinjaCrawler/commit/2258472d3a9a0bd55e57459014258b0a9fbc0a42))
* **release:** isolate package release flows and add tag recovery ([#128](https://github.com/JustShinobi/NinjaCrawler/issues/128)) ([0f0f5bc](https://github.com/JustShinobi/NinjaCrawler/commit/0f0f5bc74a6975abfb3bdb567f44326cf6fb084d))
* **release:** keep Cargo lock version aligned ([e66e4d0](https://github.com/JustShinobi/NinjaCrawler/commit/e66e4d0c810c869e5c5f709a4313fc79f0c6e99a))
* **release:** keep historical recovery self-contained ([7b84214](https://github.com/JustShinobi/NinjaCrawler/commit/7b84214e08dbe56c3300d3170b4792657d985c58))
* **release:** make post-publish maintenance resilient ([906b74c](https://github.com/JustShinobi/NinjaCrawler/commit/906b74cc7ee4a1dd0e86e0f5826e29cb74f61f8e))
* **release:** overlay trusted tooling for recovery ([3fba695](https://github.com/JustShinobi/NinjaCrawler/commit/3fba69566fe066555a864140431013913e9ef1d1))
* **release:** prevent empty promotion pull requests ([#141](https://github.com/JustShinobi/NinjaCrawler/issues/141)) ([f23e662](https://github.com/JustShinobi/NinjaCrawler/commit/f23e662b1fbd5728f4ec9972d5e54fb34c2e8d92))
* **release:** prevent version regressions in promotion ([ef487d0](https://github.com/JustShinobi/NinjaCrawler/commit/ef487d061b7728397ef4ddfb7c7c84ca014a9f4e))
* **release:** prevent version regressions in promotion ([ef487d0](https://github.com/JustShinobi/NinjaCrawler/commit/ef487d061b7728397ef4ddfb7c7c84ca014a9f4e))
* **release:** prevent version regressions in promotion ([c061f2a](https://github.com/JustShinobi/NinjaCrawler/commit/c061f2a72d749ced23c95d4cd27c3f91e55ea06b))
* **release:** regenerate sibling release pull requests ([#139](https://github.com/JustShinobi/NinjaCrawler/issues/139)) ([6ab297d](https://github.com/JustShinobi/NinjaCrawler/commit/6ab297db938e7ff13ff098a06e6aa931fd9ccffa))
* **release:** use labeled promotion PR directly ([5575879](https://github.com/JustShinobi/NinjaCrawler/commit/5575879a8c39cabe7c63a7efeadd48d957313e49))
* **release:** use Linux release notes path ([4d9f2f5](https://github.com/JustShinobi/NinjaCrawler/commit/4d9f2f564f3f9e175beea41353fb196bde9f3cfa))
* **single-videos:** window ACL + toolbar entry; queue TikTok story downloads ([5040066](https://github.com/JustShinobi/NinjaCrawler/commit/50400666eb5c73e9d0078c584f96e4daf45f213d))
* **sync:** prevent zero-byte media artifacts ([#154](https://github.com/JustShinobi/NinjaCrawler/issues/154)) ([5b42b5d](https://github.com/JustShinobi/NinjaCrawler/commit/5b42b5da035459899cc892f716104f4cc9a92019))
* **tiktok:** preserve sync history on transient empty provider listings ([#92](https://github.com/JustShinobi/NinjaCrawler/issues/92)) ([f3bff77](https://github.com/JustShinobi/NinjaCrawler/commit/f3bff770b25ec8f3ec982ef934137acfc24f70d9))
* **tiktok:** purge bogus timeline rows for liked media ([#117](https://github.com/JustShinobi/NinjaCrawler/issues/117)) ([5474fc4](https://github.com/JustShinobi/NinjaCrawler/commit/5474fc43bb40a212de050a8bd47801f3d55b0b4a))
* **tiktok:** reject audio-only video downloads ([#185](https://github.com/JustShinobi/NinjaCrawler/issues/185)) ([6e3deae](https://github.com/JustShinobi/NinjaCrawler/commit/6e3deaed49aad17a9952a95f5efe53fa2cc3915b))
* **workspace:** include all provider save paths in filter ([#120](https://github.com/JustShinobi/NinjaCrawler/issues/120)) ([a304f09](https://github.com/JustShinobi/NinjaCrawler/commit/a304f096b04fd06a3e84eecd85eb9d25763f719a))
* **workspace:** preserve existing media paths on account path change ([#121](https://github.com/JustShinobi/NinjaCrawler/issues/121)) ([f732637](https://github.com/JustShinobi/NinjaCrawler/commit/f732637b975fe6c0c3121798e52eef237fb5800d))
* **workspace:** prevent profile context menu from overflowing window ([4330085](https://github.com/JustShinobi/NinjaCrawler/commit/433008594b8a94bf573b34ecafc7a7984f74de89))
* **workspace:** prevent profile context menu from overflowing window ([34a8296](https://github.com/JustShinobi/NinjaCrawler/commit/34a8296e294e11d84a4a69a8f8fe704e28f1246a))


### Performance Improvements

* **profile-view:** virtualize large profiles with progressive rendering ([9627874](https://github.com/JustShinobi/NinjaCrawler/commit/9627874db018dc96a8bdc53317696906cddf622f))
* **profile-view:** virtualize large profiles with progressive rendering ([3d89906](https://github.com/JustShinobi/NinjaCrawler/commit/3d899060e03ccce50682ee7c0e5fbced4d05f9c6))
* **workspace:** optimize profile list virtualization and media memory usage ([#116](https://github.com/JustShinobi/NinjaCrawler/issues/116)) ([4027f43](https://github.com/JustShinobi/NinjaCrawler/commit/4027f43230e5b72165aa6c09d1c4743c848e116f))


### Code Refactoring

* drop the Reddit provider from the app core ([41be930](https://github.com/JustShinobi/NinjaCrawler/commit/41be9309f91f34561332644f0f2cb32ae28724aa))

## [0.23.0](https://github.com/JustShinobi/NinjaCrawler/compare/v0.22.0...v0.23.0) (2026-07-16)


### Features

* **twitter:** warn when account cannot view sensitive media ([#43](https://github.com/JustShinobi/NinjaCrawler/issues/43)) ([a3731e3](https://github.com/JustShinobi/NinjaCrawler/commit/a3731e3338559d51b71d0c446376ba857ac7d6eb))

## [0.22.0](https://github.com/JustShinobi/NinjaCrawler/compare/v0.21.0...v0.22.0) (2026-07-16)


### Features

* **companion:** stage updates in AppData and detect first Instagram story ([#22](https://github.com/JustShinobi/NinjaCrawler/issues/22)) ([ad7577c](https://github.com/JustShinobi/NinjaCrawler/commit/ad7577c4a311a608fdebc182d87f83bcfa8314d9))

## [0.21.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.20.0...v0.21.0) (2026-07-16)


### Features

* **windows:** migrate remaining Tauri windows to WindowShell ([#11](https://github.com/MetalDevOps/NinjaCrawler/issues/11)) ([87d5155](https://github.com/MetalDevOps/NinjaCrawler/commit/87d5155e6dcbb4ed70edd85ead45f36a108fbf32))

## [0.20.0](https://github.com/MetalDevOps/NinjaCrawler-Temp/compare/v0.19.2...v0.20.0) (2026-07-16)


### Features

* **windows:** migrate utility windows to custom visual shell ([#2](https://github.com/MetalDevOps/NinjaCrawler-Temp/issues/2)) ([6dd8c63](https://github.com/MetalDevOps/NinjaCrawler-Temp/commit/6dd8c63c6f82bb26de7718362b12685b1f67c6b0))

## [0.19.2](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.19.1...v0.19.2) (2026-07-15)


### Bug Fixes

* **instagram:** restore profile note sync ([#212](https://github.com/MetalDevOps/NinjaCrawler/issues/212)) ([106b200](https://github.com/MetalDevOps/NinjaCrawler/commit/106b20082febbc9bd04a8840325293686c07e08b))

## [0.19.1](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.19.0...v0.19.1) (2026-07-15)


### Bug Fixes

* **release:** handle missing draft releases safely ([#206](https://github.com/MetalDevOps/NinjaCrawler/issues/206)) ([2258472](https://github.com/MetalDevOps/NinjaCrawler/commit/2258472d3a9a0bd55e57459014258b0a9fbc0a42))

## [0.19.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.18.2...v0.19.0) (2026-07-15)


### Features

* **workspace:** establish brand visual system and custom window shell ([#193](https://github.com/MetalDevOps/NinjaCrawler/issues/193)) ([1d91f7f](https://github.com/MetalDevOps/NinjaCrawler/commit/1d91f7feaea5090a1d22779f5b75a7eb85a8723a))

## [0.18.2](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.18.1...v0.18.2) (2026-07-14)


### Bug Fixes

* **tiktok:** reject audio-only video downloads ([#185](https://github.com/MetalDevOps/NinjaCrawler/issues/185)) ([6e3deae](https://github.com/MetalDevOps/NinjaCrawler/commit/6e3deaed49aad17a9952a95f5efe53fa2cc3915b))

## [0.18.1](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.18.0...v0.18.1) (2026-07-14)


### Bug Fixes

* **release:** keep historical recovery self-contained ([7b84214](https://github.com/MetalDevOps/NinjaCrawler/commit/7b84214e08dbe56c3300d3170b4792657d985c58))
* **release:** make post-publish maintenance resilient ([906b74c](https://github.com/MetalDevOps/NinjaCrawler/commit/906b74cc7ee4a1dd0e86e0f5826e29cb74f61f8e))
* **release:** overlay trusted tooling for recovery ([3fba695](https://github.com/MetalDevOps/NinjaCrawler/commit/3fba69566fe066555a864140431013913e9ef1d1))
* **release:** prevent version regressions in promotion ([ef487d0](https://github.com/MetalDevOps/NinjaCrawler/commit/ef487d061b7728397ef4ddfb7c7c84ca014a9f4e))
* **release:** prevent version regressions in promotion ([ef487d0](https://github.com/MetalDevOps/NinjaCrawler/commit/ef487d061b7728397ef4ddfb7c7c84ca014a9f4e))
* **release:** prevent version regressions in promotion ([c061f2a](https://github.com/MetalDevOps/NinjaCrawler/commit/c061f2a72d749ced23c95d4cd27c3f91e55ea06b))
* **release:** use Linux release notes path ([4d9f2f5](https://github.com/MetalDevOps/NinjaCrawler/commit/4d9f2f564f3f9e175beea41353fb196bde9f3cfa))

## [0.18.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.17.1...v0.18.0) (2026-07-14)


### Features

* **release:** cross-build thin Windows distribution ([#160](https://github.com/MetalDevOps/NinjaCrawler/issues/160)) ([408bb96](https://github.com/MetalDevOps/NinjaCrawler/commit/408bb96b820f6a6015f46dea4590845ff7692ffa))


### Bug Fixes

* **release:** use labeled promotion PR directly ([5575879](https://github.com/MetalDevOps/NinjaCrawler/commit/5575879a8c39cabe7c63a7efeadd48d957313e49))

## [0.17.1](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.17.0...v0.17.1) (2026-07-14)


### Bug Fixes

* **sync:** prevent zero-byte media artifacts ([#154](https://github.com/MetalDevOps/NinjaCrawler/issues/154)) ([5b42b5d](https://github.com/MetalDevOps/NinjaCrawler/commit/5b42b5da035459899cc892f716104f4cc9a92019))

## [0.17.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.16.1...v0.17.0) (2026-07-13)


### Features

* **release:** add app update checker and release back-sync ([#140](https://github.com/MetalDevOps/NinjaCrawler/issues/140)) ([58458d5](https://github.com/MetalDevOps/NinjaCrawler/commit/58458d5bfad08a95aede5f2bda377833969af000))


### Bug Fixes

* **release:** keep Cargo lock version aligned ([e66e4d0](https://github.com/MetalDevOps/NinjaCrawler/commit/e66e4d0c810c869e5c5f709a4313fc79f0c6e99a))

## [0.16.1](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.16.0...v0.16.1) (2026-07-13)


### Bug Fixes

* **companion:** detect initial Instagram story by resolving live tab URL ([#125](https://github.com/MetalDevOps/NinjaCrawler/issues/125)) ([ee036cf](https://github.com/MetalDevOps/NinjaCrawler/commit/ee036cf20388dd278823eb8d92c8494a04c7bddd))
* **queue:** dispatch image thumbnail generation correctly ([#127](https://github.com/MetalDevOps/NinjaCrawler/issues/127)) ([44ed4e3](https://github.com/MetalDevOps/NinjaCrawler/commit/44ed4e3977eaf5323094cac935bc863b752bef57))
* **release:** authenticate recovery with repository PAT ([#130](https://github.com/MetalDevOps/NinjaCrawler/issues/130)) ([be6eed0](https://github.com/MetalDevOps/NinjaCrawler/commit/be6eed0618427875cd42053fad3e03a3c6a15b06))
* **release:** isolate package release flows and add tag recovery ([#128](https://github.com/MetalDevOps/NinjaCrawler/issues/128)) ([0f0f5bc](https://github.com/MetalDevOps/NinjaCrawler/commit/0f0f5bc74a6975abfb3bdb567f44326cf6fb084d))

## [0.16.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.15.0...v0.16.0) (2026-07-13)


### Features

* **queue:** add media path migration cancellation and detailed progress ([#123](https://github.com/MetalDevOps/NinjaCrawler/issues/123)) ([95b97b7](https://github.com/MetalDevOps/NinjaCrawler/commit/95b97b75def12c4f1f27d53193899e925c897dfd))
* **workspace:** process media path migrations in a background queue ([#119](https://github.com/MetalDevOps/NinjaCrawler/issues/119)) ([07cc3a2](https://github.com/MetalDevOps/NinjaCrawler/commit/07cc3a2ee5a27536c09b4a8f68f9c8090cef640e))


### Bug Fixes

* **tiktok:** purge bogus timeline rows for liked media ([#117](https://github.com/MetalDevOps/NinjaCrawler/issues/117)) ([5474fc4](https://github.com/MetalDevOps/NinjaCrawler/commit/5474fc43bb40a212de050a8bd47801f3d55b0b4a))
* **workspace:** include all provider save paths in filter ([#120](https://github.com/MetalDevOps/NinjaCrawler/issues/120)) ([a304f09](https://github.com/MetalDevOps/NinjaCrawler/commit/a304f096b04fd06a3e84eecd85eb9d25763f719a))
* **workspace:** preserve existing media paths on account path change ([#121](https://github.com/MetalDevOps/NinjaCrawler/issues/121)) ([f732637](https://github.com/MetalDevOps/NinjaCrawler/commit/f732637b975fe6c0c3121798e52eef237fb5800d))


### Performance Improvements

* **workspace:** optimize profile list virtualization and media memory usage ([#116](https://github.com/MetalDevOps/NinjaCrawler/issues/116)) ([4027f43](https://github.com/MetalDevOps/NinjaCrawler/commit/4027f43230e5b72165aa6c09d1c4743c848e116f))

## [0.15.0](https://github.com/MetalDevOps/NinjaCrawler/compare/v0.14.0...v0.15.0) (2026-07-11)


### Features

* **companion:** harden account capture and block incomplete session imports ([#102](https://github.com/MetalDevOps/NinjaCrawler/issues/102)) ([5e53316](https://github.com/MetalDevOps/NinjaCrawler/commit/5e53316b94b2ea1191add5f1a551a5924987af94))
* **connectors:** implement Twitter incremental sync and rate limit holds ([#112](https://github.com/MetalDevOps/NinjaCrawler/issues/112)) ([3b8992c](https://github.com/MetalDevOps/NinjaCrawler/commit/3b8992c11e28fac153afd247616bb571e46b76ee))

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
