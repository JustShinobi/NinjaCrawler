use crate::domain::models::{
    RunSourceSyncInput, SourceEditorSeedIntent, SourceEditorWindowIntent, SourceProfile,
};
use crate::infrastructure::{desktop_runtime, source_sync_runtime, workspace_repository};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use tauri::AppHandle;

const BIND_ADDR: &str = "127.0.0.1:47219";
const API_PREFIX: &str = "/ninjacrawler-companion/v1";
const MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DetectedProfile {
    provider: String,
    handle: String,
    display_name: String,
    canonical_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompanionContext {
    app: &'static str,
    api_version: u8,
    detected_profile: Option<DetectedProfile>,
    existing_source: Option<SourceProfile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddSourceRequest {
    provider: String,
    handle: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncSourceRequest {
    source_id: String,
}

struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(BIND_ADDR) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("NinjaCrawler Companion API disabled: {error}");
                return;
            }
        };

        for stream in listener.incoming() {
            let app = app.clone();
            match stream {
                Ok(stream) => {
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(app, stream) {
                            eprintln!("NinjaCrawler Companion API request failed: {error}");
                        }
                    });
                }
                Err(error) => {
                    eprintln!("NinjaCrawler Companion API connection failed: {error}");
                }
            }
        }
    });
}

fn handle_connection(app: AppHandle, mut stream: TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;

    let request = read_request(&mut stream)?;
    let response = route_request(app, request);
    stream
        .write_all(&response)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;

    loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("Empty HTTP request.".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            header_end = index;
            break;
        }
        if buffer.len() > MAX_BODY_BYTES {
            return Err("HTTP request is too large.".to_string());
        }
    }

    let header_bytes = &buffer[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "Missing HTTP request line.".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "Missing HTTP method.".to_string())?
        .to_string();
    let target = request_parts
        .next()
        .ok_or_else(|| "Missing HTTP target.".to_string())?;
    let target = target.to_string();

    let mut content_length = 0_usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid Content-Length header.".to_string())?;
            }
        }
    }
    if content_length > MAX_BODY_BYTES {
        return Err("HTTP request body is too large.".to_string());
    }

    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let (path, query) = split_target(&target);
    let body = buffer
        .get(body_start..body_start + content_length)
        .unwrap_or_default()
        .to_vec();

    Ok(HttpRequest {
        method,
        path,
        query,
        body,
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in query_text.split('&').filter(|entry| !entry.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode(key), percent_decode(value));
    }
    (path.to_string(), query)
}

fn route_request(app: AppHandle, request: HttpRequest) -> Vec<u8> {
    if request.method.eq_ignore_ascii_case("OPTIONS") {
        return empty_response(204);
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", path) if path == format!("{API_PREFIX}/health") => json_response(
            200,
            &json!({
                "app": "NinjaCrawler",
                "companion": "NinjaCrawler Companion",
                "apiVersion": 1,
                "status": "ok"
            }),
        ),
        ("GET", path) if path == format!("{API_PREFIX}/context") => {
            let url = request.query.get("url").map(String::as_str);
            match build_context(url) {
                Ok(context) => json_response(200, &context),
                Err(error) => error_response(500, &error),
            }
        }
        ("POST", path) if path == format!("{API_PREFIX}/source") => {
            match parse_json::<AddSourceRequest>(&request.body)
                .and_then(|input| add_source(app, input))
            {
                Ok(payload) => json_response(200, &payload),
                Err(error) => error_response(400, &error),
            }
        }
        ("POST", path) if path == format!("{API_PREFIX}/sync") => {
            match parse_json::<SyncSourceRequest>(&request.body)
                .and_then(|input| sync_source(app, input))
            {
                Ok(payload) => json_response(200, &payload),
                Err(error) => error_response(400, &error),
            }
        }
        _ => error_response(404, "Unknown NinjaCrawler Companion API endpoint."),
    }
}

fn build_context(url: Option<&str>) -> Result<CompanionContext, String> {
    let snapshot = workspace_repository::bootstrap_workspace()?;
    let detected_profile = url.and_then(detect_profile_from_url);
    let existing_source = detected_profile.as_ref().and_then(|detected| {
        find_source(&snapshot.sources, &detected.provider, &detected.handle).cloned()
    });

    Ok(CompanionContext {
        app: "NinjaCrawler",
        api_version: 1,
        detected_profile,
        existing_source,
    })
}

fn add_source(app: AppHandle, input: AddSourceRequest) -> Result<serde_json::Value, String> {
    let provider = normalize_provider(&input.provider)?;
    let handle = normalize_handle(&input.handle);
    if handle.is_empty() {
        return Err("Profile handle is required.".to_string());
    }

    let display_name = input
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| handle.trim_start_matches('@'))
        .to_string();

    desktop_runtime::open_source_editor_window(
        &app,
        Some(SourceEditorWindowIntent {
            source_id: None,
            preferred_provider: Some(provider.clone()),
            preferred_account_id: None,
            seed: Some(SourceEditorSeedIntent {
                provider: provider.clone(),
                handle: handle.clone(),
                display_name: display_name.clone(),
            }),
        }),
    )?;

    Ok(json!({
        "opened": true,
        "provider": provider,
        "handle": handle,
        "displayName": display_name
    }))
}

fn sync_source(app: AppHandle, input: SyncSourceRequest) -> Result<serde_json::Value, String> {
    let source_id = input.source_id.trim();
    if source_id.is_empty() {
        return Err("Source id is required.".to_string());
    }

    let snapshot = source_sync_runtime::enqueue_source_sync(
        &app,
        RunSourceSyncInput {
            id: source_id.to_string(),
            trigger: Some("chrome_extension".to_string()),
            run_mode: None,
            sync_options_override: None,
        },
    )?;

    Ok(json!({
        "snapshot": snapshot,
        "queued": true
    }))
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|error| format!("Invalid JSON payload: {error}"))
}

fn find_source<'a>(
    sources: &'a [SourceProfile],
    provider: &str,
    handle: &str,
) -> Option<&'a SourceProfile> {
    let key = canonical_profile_key(provider, handle);
    sources.iter().find(|source| {
        source.provider == provider && canonical_profile_key(provider, &source.handle) == key
    })
}

fn detect_profile_from_url(url: &str) -> Option<DetectedProfile> {
    let parsed = parse_url(url)?;
    let host = parsed.host.trim_start_matches("www.").to_ascii_lowercase();
    let segments: Vec<&str> = parsed
        .path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();

    let (provider, handle) = if host == "instagram.com" || host.ends_with(".instagram.com") {
        let first = segments.first().copied()?;
        if matches!(
            first,
            "accounts" | "direct" | "explore" | "p" | "reel" | "reels" | "stories" | "tv"
        ) {
            return None;
        }
        ("instagram", first)
    } else if host == "x.com" || host == "twitter.com" || host.ends_with(".twitter.com") {
        let first = segments.first().copied()?;
        if matches!(
            first,
            "compose"
                | "explore"
                | "home"
                | "i"
                | "intent"
                | "login"
                | "messages"
                | "notifications"
                | "search"
                | "settings"
                | "share"
        ) {
            return None;
        }
        ("twitter", first)
    } else if host == "tiktok.com" || host.ends_with(".tiktok.com") {
        let first = segments.first().copied()?;
        if !first.starts_with('@') {
            return None;
        }
        ("tiktok", first)
    } else if host == "reddit.com" || host.ends_with(".reddit.com") {
        if segments.len() < 2 || !(segments[0] == "user" || segments[0] == "u") {
            return None;
        }
        ("reddit", segments[1])
    } else {
        return None;
    };

    let handle = normalize_handle(handle);
    if handle.is_empty() {
        return None;
    }

    Some(DetectedProfile {
        provider: provider.to_string(),
        display_name: handle.trim_start_matches('@').to_string(),
        canonical_key: canonical_profile_key(provider, &handle),
        handle,
    })
}

struct ParsedUrl {
    host: String,
    path: String,
}

fn parse_url(url: &str) -> Option<ParsedUrl> {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let without_fragment = without_scheme
        .split_once('#')
        .map(|(left, _)| left)
        .unwrap_or(without_scheme);
    let without_query = without_fragment
        .split_once('?')
        .map(|(left, _)| left)
        .unwrap_or(without_fragment);
    let (host, path) = without_query
        .split_once('/')
        .map(|(host, path)| (host, format!("/{path}")))
        .unwrap_or((without_query, "/".to_string()));
    let host = host.split_once(':').map(|(host, _)| host).unwrap_or(host);
    if host.trim().is_empty() {
        return None;
    }
    Some(ParsedUrl {
        host: host.to_string(),
        path,
    })
}

fn normalize_provider(provider: &str) -> Result<String, String> {
    let normalized = provider.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "instagram" | "tiktok" | "reddit" | "twitter" => Ok(normalized),
        _ => Err(format!("Unsupported provider '{provider}'.")),
    }
}

fn normalize_handle(handle: &str) -> String {
    let trimmed = handle
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_matches('/');
    let candidate = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let candidate = candidate.split('?').next().unwrap_or(candidate);
    let candidate = percent_decode(candidate).trim().to_string();
    if candidate.is_empty() {
        return String::new();
    }
    if candidate.starts_with('@') {
        candidate
    } else {
        format!("@{candidate}")
    }
}

fn canonical_profile_key(provider: &str, handle: &str) -> String {
    let handle = normalize_handle(handle)
        .trim_start_matches('@')
        .to_ascii_lowercase();
    format!("{}:{handle}", provider.to_ascii_lowercase())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
                index += 3;
                continue;
            }
        }
        output.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn json_response<T: Serialize>(status: u16, payload: &T) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    response(status, "application/json; charset=utf-8", body)
}

fn error_response(status: u16, message: &str) -> Vec<u8> {
    json_response(
        status,
        &json!({
            "error": message
        }),
    )
}

fn empty_response(status: u16) -> Vec<u8> {
    response(status, "text/plain; charset=utf-8", Vec::new())
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        body.len(),
    );

    let mut response = headers.into_bytes();
    response.extend_from_slice(&body);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_profile_urls() {
        let cases = [
            (
                "https://www.instagram.com/example.profile/",
                "instagram",
                "@example.profile",
            ),
            (
                "https://x.com/example_user/media",
                "twitter",
                "@example_user",
            ),
            (
                "https://twitter.com/example_user",
                "twitter",
                "@example_user",
            ),
            (
                "https://www.tiktok.com/@example/video/123",
                "tiktok",
                "@example",
            ),
            ("https://www.reddit.com/user/example/", "reddit", "@example"),
            ("https://www.reddit.com/u/example/", "reddit", "@example"),
        ];

        for (url, provider, handle) in cases {
            let detected = detect_profile_from_url(url).expect(url);
            assert_eq!(detected.provider, provider);
            assert_eq!(detected.handle, handle);
        }
    }

    #[test]
    fn ignores_non_profile_urls() {
        let cases = [
            "https://www.instagram.com/reel/123/",
            "https://x.com/home",
            "https://www.tiktok.com/tag/rust",
            "https://www.reddit.com/r/rust/",
        ];

        for url in cases {
            assert!(detect_profile_from_url(url).is_none(), "{url}");
        }
    }
}
