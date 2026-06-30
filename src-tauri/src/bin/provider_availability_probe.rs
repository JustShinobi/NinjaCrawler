use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use ninjacrawler_lib::domain::models::{
    ProviderAccountUpsert, SourceAvailabilityCheckResult, SourceProfile, SourceProfileUpsert,
    SourceSyncOptions,
};
use ninjacrawler_lib::infrastructure::{instagram_connector, workspace_repository};
use serde::Serialize;
use uuid::Uuid;

const INSTAGRAM_PUBLIC_APP_ID: &str = "936619743392459";
const INSTAGRAM_PUBLIC_ASBD_ID: &str = "129477";
const INSTAGRAM_PUBLIC_IG_CLAIM: &str = "0";
const INSTAGRAM_PUBLIC_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

#[derive(Debug)]
struct Args {
    profiles: Vec<String>,
    json_out: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
}

#[derive(Serialize)]
struct ProbeOutput {
    requested_profiles: Vec<String>,
    source_ids: Vec<String>,
    availability: SourceAvailabilityCheckResult,
    provider_results: Vec<ProviderProbeResult>,
    persisted_sync_problems: Vec<PersistedSyncProblem>,
}

#[derive(Serialize)]
struct ProviderProbeResult {
    source_id: String,
    handle: String,
    resolved_username: Option<String>,
    error: Option<String>,
    http_status: Option<u16>,
}

#[derive(Serialize)]
struct PersistedSyncProblem {
    source_id: String,
    handle: String,
    sync_problem_code: Option<String>,
    sync_problem_message: Option<String>,
    ready_for_download: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("provider_availability_probe failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if args.profiles.is_empty() {
        return Err("No profiles provided. Use --profiles <url-or-handle> [...]".to_string());
    }

    if let Some(root) = args.workspace_root.as_deref() {
        let local_app_data = root.join("localappdata");
        let user_profile = root.join("userprofile");
        fs::create_dir_all(&local_app_data).map_err(|error| error.to_string())?;
        fs::create_dir_all(&user_profile).map_err(|error| error.to_string())?;
        unsafe {
            env::set_var("LOCALAPPDATA", local_app_data);
            env::set_var("USERPROFILE", user_profile);
        }
    }

    let handles: Vec<String> = args
        .profiles
        .iter()
        .map(|value| parse_instagram_handle(value))
        .collect::<Result<Vec<_>, _>>()?;

    let probe_account_id = format!("probe-account-{}", Uuid::new_v4());
    workspace_repository::upsert_provider_account(ProviderAccountUpsert {
        id: Some(probe_account_id.clone()),
        provider: "instagram".to_string(),
        display_name: "Provider Probe".to_string(),
        auth_mode: "imported_session".to_string(),
        auth_state: "degraded".to_string(),
        capabilities: vec!["profile".to_string()],
        last_validated_at: None,
    })?;

    let mut source_ids = Vec::<String>::with_capacity(handles.len());
    for handle in &handles {
        let source_id = format!("probe-{}", Uuid::new_v4());
        workspace_repository::upsert_source_profile(SourceProfileUpsert {
            id: Some(source_id.clone()),
            provider: "instagram".to_string(),
            source_kind: "profile".to_string(),
            handle: handle.clone(),
            display_name: handle.clone(),
            account_id: Some(probe_account_id.clone()),
            group_id: None,
            labels: vec!["provider-probe".to_string()],
            ready_for_download: true,
            sync_options: SourceSyncOptions::default(),
            remote_state: Some("exists".to_string()),
            is_subscription: Some(false),
        })?;
        source_ids.push(source_id);
    }

    let availability =
        workspace_repository::check_source_availability(source_ids.clone(), None)?;
    let provider_results = source_ids
        .iter()
        .zip(handles.iter())
        .map(|(source_id, handle)| probe_provider_identity(source_id, handle))
        .collect::<Vec<_>>();
    let persisted_sync_problems =
        collect_sync_problems(&availability.snapshot.sources, &source_ids);

    let output = ProbeOutput {
        requested_profiles: args.profiles,
        source_ids,
        availability,
        provider_results,
        persisted_sync_problems,
    };

    let json = serde_json::to_string_pretty(&output)
        .map_err(|error| format!("JSON encode failed: {error}"))?;
    println!("{json}");
    if let Some(path) = args.json_out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &json).map_err(|error| error.to_string())?;
        eprintln!("Saved probe output to {}", path.display());
    }

    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut profiles = Vec::<String>::new();
    let mut json_out: Option<PathBuf> = None;
    let mut workspace_root: Option<PathBuf> = None;

    let mut iter = env::args().skip(1).peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--profiles" => {
                while let Some(next) = iter.peek() {
                    if next.starts_with("--") {
                        break;
                    }
                    if let Some(value) = iter.next() {
                        profiles.push(value);
                    }
                }
            }
            "--profile" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value after --profile".to_string())?;
                profiles.push(value);
            }
            "--json-out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value after --json-out".to_string())?;
                json_out = Some(PathBuf::from(value));
            }
            "--workspace-root" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value after --workspace-root".to_string())?;
                workspace_root = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            unknown => {
                return Err(format!(
                    "Unknown argument '{unknown}'. Use --help for usage."
                ));
            }
        }
    }

    Ok(Args {
        profiles,
        json_out,
        workspace_root,
    })
}

fn print_help() {
    println!(
        "provider_availability_probe\n\
         \n\
         Usage:\n\
           cargo run --manifest-path src-tauri/Cargo.toml --bin provider_availability_probe -- \\\n\
             --profiles <url-or-handle> [more...] [--json-out <path>] [--workspace-root <path>]\n\
         \n\
         Options:\n\
           --profiles        One or more Instagram profile URLs or handles.\n\
           --profile         Single Instagram profile URL or handle (repeatable).\n\
           --json-out        Optional path to save JSON output.\n\
           --workspace-root  Optional isolated root used to set LOCALAPPDATA/USERPROFILE.\n\
           --help            Show this help.\n"
    );
}

fn parse_instagram_handle(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Profile value cannot be empty.".to_string());
    }

    if trimmed.contains("instagram.com") {
        let without_protocol = trimmed
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let path_part = without_protocol
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or("");
        let candidate = path_part
            .split('/')
            .find(|segment| !segment.trim().is_empty())
            .ok_or_else(|| format!("Could not parse Instagram handle from URL '{trimmed}'."))?;
        let normalized = sanitize_handle(candidate);
        if normalized.is_empty() {
            return Err(format!(
                "Could not normalize Instagram handle from URL '{trimmed}'."
            ));
        }
        return Ok(normalized);
    }

    let normalized = sanitize_handle(trimmed);
    if normalized.is_empty() {
        return Err(format!("Could not normalize Instagram handle '{trimmed}'."));
    }
    Ok(normalized)
}

fn sanitize_handle(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('@')
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn probe_provider_identity(source_id: &str, handle: &str) -> ProviderProbeResult {
    let request = build_instagram_identity_probe_request(handle);
    match instagram_connector::resolve_profile_identity(&request, None) {
        Ok(identity) => ProviderProbeResult {
            source_id: source_id.to_string(),
            handle: handle.to_string(),
            resolved_username: Some(identity.username),
            error: None,
            http_status: None,
        },
        Err(error) => ProviderProbeResult {
            source_id: source_id.to_string(),
            handle: handle.to_string(),
            http_status: extract_http_status_code_from_message(&error),
            resolved_username: None,
            error: Some(error),
        },
    }
}

fn build_instagram_identity_probe_request(
    username: &str,
) -> instagram_connector::InstagramConnectorRequest {
    instagram_connector::InstagramConnectorRequest {
        username: username.to_string(),
        cookies: Vec::new(),
        headers: instagram_connector::InstagramAuthHeaders {
            app_id: Some(INSTAGRAM_PUBLIC_APP_ID.to_string()),
            asbd_id: Some(INSTAGRAM_PUBLIC_ASBD_ID.to_string()),
            ig_www_claim: Some(INSTAGRAM_PUBLIC_IG_CLAIM.to_string()),
            user_agent: Some(INSTAGRAM_PUBLIC_USER_AGENT.to_string()),
            ..Default::default()
        },
        profile_root: PathBuf::new(),
        saved_posts_root: PathBuf::new(),
        ledger_post_keys: HashSet::new(),
        deleted_post_keys: HashSet::new(),
        existing_media_keys: HashSet::new(),
        ledger_media_keys: HashSet::new(),
        existing_relative_paths: HashSet::new(),
        ledger_relative_paths: HashSet::new(),
        sections: instagram_connector::InstagramSectionSelection::default(),
        use_gql: true,
        download_saved_posts: false,
        post_page_size: 12,
        skip_errors: true,
        ignore_stories_560_errors: true,
        request_delay_ms: 0,
        timeout_secs: 20,
        download_images: false,
        download_videos: false,
        extract_image_from_video: instagram_connector::InstagramSectionSelection::default(),
        place_extracted_image_into_video_folder: false,
        download_text: false,
        download_text_posts: false,
        text_special_folder: false,
        get_user_media_only: false,
        missing_only: false,
        date_from_timestamp: None,
        date_to_timestamp: None,
        media_file_naming_mode: instagram_connector::InstagramMediaFileNamingMode::PresetNewDefault,
        media_file_naming_template: None,
        target_story_media_id: None,
    }
}

fn extract_http_status_code_from_message(error: &str) -> Option<u16> {
    let bytes = error.as_bytes();
    for index in 0..bytes.len().saturating_sub(2) {
        if !bytes[index].is_ascii_digit()
            || !bytes[index + 1].is_ascii_digit()
            || !bytes[index + 2].is_ascii_digit()
        {
            continue;
        }

        let has_left_boundary = index == 0 || !bytes[index - 1].is_ascii_digit();
        let has_right_boundary = index + 3 >= bytes.len() || !bytes[index + 3].is_ascii_digit();
        if !has_left_boundary || !has_right_boundary {
            continue;
        }

        let code = std::str::from_utf8(&bytes[index..index + 3])
            .ok()
            .and_then(|value| value.parse::<u16>().ok())?;
        if (100..=599).contains(&code) {
            return Some(code);
        }
    }
    None
}

fn collect_sync_problems(
    sources: &[SourceProfile],
    source_ids: &[String],
) -> Vec<PersistedSyncProblem> {
    let source_id_set = source_ids.iter().collect::<HashSet<_>>();
    sources
        .iter()
        .filter(|source| source_id_set.contains(&source.id))
        .map(|source| PersistedSyncProblem {
            source_id: source.id.clone(),
            handle: source.handle.clone(),
            sync_problem_code: source.sync_problem_code.clone(),
            sync_problem_message: source.sync_problem_message.clone(),
            ready_for_download: source.ready_for_download,
        })
        .collect::<Vec<_>>()
}
