use crate::domain::models::ProviderDescriptor;

#[derive(Clone, Copy)]
pub enum SourceSyncArgumentMode {
    GalleryDlDirectory,
    YtDlpDirectory,
}

#[derive(Clone, Copy)]
pub struct SourceSyncRuntime {
    pub tool_setting_key: &'static str,
    pub default_executable: &'static str,
    pub argument_mode: SourceSyncArgumentMode,
    pub degraded_capabilities: &'static [&'static str],
}

pub trait ProviderRuntime: Sync {
    fn key(&self) -> &'static str;
    fn descriptor(&self) -> ProviderDescriptor;
    fn source_sync_runtime(&self) -> Option<SourceSyncRuntime>;
}

struct StaticProviderRuntime {
    key: &'static str,
    display_name: &'static str,
    auth_modes: &'static [&'static str],
    supports_multi_account: bool,
    source_kinds: &'static [&'static str],
    default_capabilities: &'static [&'static str],
    notes: &'static str,
    source_sync: Option<SourceSyncRuntime>,
}

impl ProviderRuntime for StaticProviderRuntime {
    fn key(&self) -> &'static str {
        self.key
    }

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: self.key.into(),
            display_name: self.display_name.into(),
            auth_modes: self
                .auth_modes
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            supports_multi_account: self.supports_multi_account,
            source_kinds: self
                .source_kinds
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            default_capabilities: self
                .default_capabilities
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            notes: self.notes.into(),
        }
    }

    fn source_sync_runtime(&self) -> Option<SourceSyncRuntime> {
        self.source_sync
    }
}

static INSTAGRAM_RUNTIME: StaticProviderRuntime = StaticProviderRuntime {
    key: "instagram",
    display_name: "Instagram",
    auth_modes: &["imported_session"],
    supports_multi_account: true,
    source_kinds: &["profile"],
    default_capabilities: &["posts", "reels", "stories", "saved_posts"],
    notes: "Primary V1 provider. Multi-account auth is mandatory.",
    source_sync: Some(SourceSyncRuntime {
        tool_setting_key: "tool.gallery-dl.path",
        default_executable: "gallery-dl",
        argument_mode: SourceSyncArgumentMode::GalleryDlDirectory,
        degraded_capabilities: &["saved_posts"],
    }),
};

static TIKTOK_RUNTIME: StaticProviderRuntime = StaticProviderRuntime {
    key: "tiktok",
    display_name: "TikTok",
    auth_modes: &["imported_session"],
    supports_multi_account: true,
    source_kinds: &["profile"],
    default_capabilities: &["videos", "photos"],
    notes: "Primary V1 provider. Video and photo backends may diverge by capability.",
    source_sync: Some(SourceSyncRuntime {
        tool_setting_key: "tool.yt-dlp.path",
        default_executable: "yt-dlp",
        argument_mode: SourceSyncArgumentMode::YtDlpDirectory,
        degraded_capabilities: &["photos"],
    }),
};

static YOUTUBE_RUNTIME: StaticProviderRuntime = StaticProviderRuntime {
    key: "youtube",
    display_name: "YouTube",
    auth_modes: &["imported_session"],
    supports_multi_account: true,
    source_kinds: &["profile"],
    default_capabilities: &["videos", "shorts"],
    notes: "Internal connector: yt-dlp enumerates the channel and NinjaCrawler downloads media.",
    source_sync: Some(SourceSyncRuntime {
        tool_setting_key: "tool.yt-dlp.path",
        default_executable: "yt-dlp",
        argument_mode: SourceSyncArgumentMode::YtDlpDirectory,
        degraded_capabilities: &[],
    }),
};

static TWITTER_RUNTIME: StaticProviderRuntime = StaticProviderRuntime {
    key: "twitter",
    display_name: "X / Twitter",
    auth_modes: &["imported_session"],
    supports_multi_account: false,
    source_kinds: &["profile"],
    default_capabilities: &["posts", "media_timeline"],
    notes: "Internal connector: gallery-dl parses the timeline, NinjaCrawler downloads media.",
    source_sync: Some(SourceSyncRuntime {
        tool_setting_key: "tool.gallery-dl.path",
        default_executable: "gallery-dl",
        argument_mode: SourceSyncArgumentMode::GalleryDlDirectory,
        degraded_capabilities: &[],
    }),
};

static VSCO_RUNTIME: StaticProviderRuntime = StaticProviderRuntime {
    key: "vsco",
    display_name: "VSCO",
    auth_modes: &["imported_session"],
    supports_multi_account: false,
    source_kinds: &["profile"],
    default_capabilities: &["gallery"],
    notes: "Internal connector: gallery-dl parses the gallery, NinjaCrawler downloads media.",
    source_sync: Some(SourceSyncRuntime {
        tool_setting_key: "tool.gallery-dl.path",
        default_executable: "gallery-dl",
        argument_mode: SourceSyncArgumentMode::GalleryDlDirectory,
        degraded_capabilities: &[],
    }),
};

static PROVIDER_REGISTRY: [&dyn ProviderRuntime; 5] = [
    &INSTAGRAM_RUNTIME,
    &TIKTOK_RUNTIME,
    &YOUTUBE_RUNTIME,
    &TWITTER_RUNTIME,
    &VSCO_RUNTIME,
];

pub fn provider_runtime(provider: &str) -> Option<&'static dyn ProviderRuntime> {
    PROVIDER_REGISTRY
        .iter()
        .copied()
        .find(|runtime| runtime.key().eq_ignore_ascii_case(provider))
}

pub fn provider_catalog() -> Vec<ProviderDescriptor> {
    PROVIDER_REGISTRY
        .iter()
        .map(|runtime| runtime.descriptor())
        .collect()
}

pub fn source_sync_runtime(provider: &str) -> Option<SourceSyncRuntime> {
    provider_runtime(provider).and_then(|runtime| runtime.source_sync_runtime())
}

#[cfg(test)]
mod tests {
    use super::{provider_catalog, source_sync_runtime};

    #[test]
    fn provider_catalog_contains_supported_v1_runtimes() {
        let catalog = provider_catalog();
        assert_eq!(catalog.len(), 5);
        assert!(catalog.iter().any(|provider| provider.key == "instagram"));
        assert!(catalog.iter().any(|provider| provider.key == "tiktok"));
        assert!(catalog.iter().any(|provider| provider.key == "youtube"));
        assert!(catalog.iter().any(|provider| provider.key == "twitter"));
        assert!(catalog.iter().any(|provider| provider.key == "vsco"));
    }

    #[test]
    fn sync_runtime_lookup_exposes_tool_bindings() {
        let instagram = source_sync_runtime("instagram").expect("instagram runtime");
        let twitter = source_sync_runtime("twitter").expect("twitter runtime");
        assert_eq!(instagram.default_executable, "gallery-dl");
        assert_eq!(twitter.default_executable, "gallery-dl");
    }
}
