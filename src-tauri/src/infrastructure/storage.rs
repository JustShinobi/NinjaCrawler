use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone)]
pub struct StorageLayout {
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub db_path: PathBuf,
    pub media_root: PathBuf,
    pub cache_root: PathBuf,
    pub connectors_root: PathBuf,
}

pub fn ensure_workspace_layout() -> std::io::Result<StorageLayout> {
    let local_app_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let user_profile = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| local_app_data.clone());

    workspace_layout_from_roots(local_app_data, user_profile)
}

pub fn workspace_layout_from_roots(
    local_app_data: PathBuf,
    user_profile: PathBuf,
) -> std::io::Result<StorageLayout> {
    let root = local_app_data.join("NinjaCrawler");
    let data_dir = root.join("data");
    let logs_dir = root.join("logs");
    let cache_root = root.join("cache");
    let connectors_root = root.join("connectors");
    let legacy_connectors_root = data_dir.join("connectors");
    let media_root = preferred_media_root(&user_profile);
    let db_path = data_dir.join("ninjacrawler.db");

    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&logs_dir)?;
    fs::create_dir_all(&cache_root)?;
    if !connectors_root.exists() && legacy_connectors_root.exists() {
        fs::rename(&legacy_connectors_root, &connectors_root)?;
    }
    fs::create_dir_all(&connectors_root)?;
    fs::create_dir_all(&media_root)?;

    Ok(StorageLayout {
        root,
        data_dir,
        logs_dir,
        db_path,
        media_root,
        cache_root,
        connectors_root,
    })
}

fn preferred_media_root(user_profile: &Path) -> PathBuf {
    user_profile.join("Pictures").join("NinjaCrawler")
}
