use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{
    webview::Cookie, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
};

use crate::infrastructure::{
    connector_debug, connector_runtime, database, session_secret_store, storage,
};
use chrono::Utc;
use uuid::Uuid;

const LIKES_WINDOW_LABEL_PREFIX: &str = "tiktok-likes";
const TIKTOK_HOME: &str = "https://www.tiktok.com/";
const JAVASCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_READY_TIMEOUT: Duration = Duration::from_secs(45);
const REQUEST_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_MAX_DURATION: Duration = Duration::from_secs(6 * 60 * 60);
const WINDOW_CREATION_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSessionEnvelope {
    #[serde(default)]
    metadata: StoredSessionMetadata,
    cookies: Vec<StoredCookie>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSessionMetadata {
    user_agent: Option<String>,
}

#[derive(Clone, Deserialize)]
struct StoredCookie {
    domain: String,
    name: String,
    value: String,
    #[serde(default = "default_cookie_path")]
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default, alias = "httpOnly", alias = "http_only")]
    http_only: bool,
}

struct StoredSession {
    account_id: String,
    account_name: String,
    source_id: String,
    source_handle: String,
    profile_root: PathBuf,
    user_agent: Option<String>,
    cookies: Vec<StoredCookie>,
    output_root: PathBuf,
    legacy_output_roots: Vec<PathBuf>,
    cookie_root: PathBuf,
    yt_dlp: String,
}

#[derive(Clone, Deserialize)]
struct LikedVideo {
    id: String,
    author: String,
    url: String,
}

pub struct TikTokLikesSourceRequest {
    pub account_id: String,
    pub source_id: String,
    pub source_handle: String,
    pub profile_root: PathBuf,
    pub item_limit: usize,
    pub incremental: bool,
    pub known_page_threshold: usize,
}

#[derive(Default)]
pub struct TikTokLikesSyncResult {
    pub discovered: usize,
    pub downloaded: usize,
    pub skipped_existing: usize,
    pub failed: usize,
    pub failures: Vec<String>,
    pub pages_read: usize,
    pub stopped_incrementally: bool,
}

pub fn run_source_sync<F, C>(
    app: &AppHandle,
    request: TikTokLikesSourceRequest,
    mut report_progress: F,
    is_cancelled: C,
) -> Result<TikTokLikesSyncResult, String>
where
    F: FnMut(Option<u32>, String, String, bool, Option<u32>),
    C: Fn() -> bool,
{
    let session = load_tiktok_session(&request)?;
    let existing = load_existing_ledger(&session.account_id)?;
    let disk_media_by_id = load_liked_media_index(&session)?;
    let mut known_present_ids = disk_media_by_id.keys().cloned().collect::<HashSet<_>>();
    known_present_ids.extend(existing.iter().filter_map(|(item_id, relative_path)| {
        resolve_stored_media(&session, relative_path).map(|_| item_id.clone())
    }));
    let baseline_complete = has_completed_full_scan(&session.account_id, &session.source_id)?;
    let incremental_active = request.incremental && baseline_complete;
    connector_debug::append_current(
        "internal.tiktok.likes",
        "system",
        "likes.sync.begin",
        format!(
            "account_id={}\nsource_id={}\noutput_root={}\nitem_limit={}\nincremental_configured={}\nbaseline_complete={}\nincremental_active={}\nknown_present={}\nknown_page_threshold={}",
            session.account_id,
            session.source_id,
            session.output_root.display(),
            request.item_limit,
            request.incremental,
            baseline_complete,
            incremental_active,
            known_present_ids.len(),
            request.known_page_threshold,
        ),
    );
    let ms_token = session
        .cookies
        .iter()
        .find(|cookie| cookie.name == "msToken" && !cookie.value.is_empty())
        .map(|cookie| cookie.value.clone())
        .ok_or_else(|| {
            "The ready TikTok session does not contain an msToken cookie.".to_string()
        })?;

    fs::create_dir_all(&session.output_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&session.cookie_root).map_err(|error| error.to_string())?;
    cleanup_stale_cookie_files(&session.cookie_root);
    if is_cancelled() {
        return Err("source sync cancelled by user".to_string());
    }
    report_progress(
        Some(0),
        "Loading liked videos".to_string(),
        "Opening an authenticated, silent TikTok session.".to_string(),
        true,
        Some(0),
    );
    let result = (|| {
        let window = create_likes_window(app, &session)?;
        let output = run_probe(
            &window,
            &ms_token,
            request.item_limit,
            &known_present_ids,
            incremental_active,
            request.known_page_threshold,
            &mut report_progress,
            &is_cancelled,
        );
        let _ = window.close();
        let output = output?;
        if output.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(output
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| output.get("statusMessage").and_then(Value::as_str))
                .unwrap_or("TikTok returned an invalid liked-videos response.")
                .to_string());
        }
        let items = serde_json::from_value::<Vec<LikedVideo>>(
            output.get("items").cloned().unwrap_or_else(|| json!([])),
        )
        .map_err(|error| format!("Could not parse TikTok liked videos: {error}"))?;
        let pages_read = output.get("pages").and_then(Value::as_u64).unwrap_or(0) as usize;
        let has_more = output
            .get("hasMore")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated = output
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stopped_incrementally = output
            .get("stoppedIncrementally")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        connector_debug::append_current(
            "internal.tiktok.likes",
            "response",
            "likes.list.complete",
            format!(
                "pages={pages_read}\nitems={}\nhas_more={has_more}\ntruncated={truncated}\nstopped_incrementally={stopped_incrementally}",
                items.len()
            ),
        );
        if !has_more && !truncated && !stopped_incrementally {
            mark_full_scan_completed(&session.account_id, &session.source_id)?;
            connector_debug::append_current(
                "internal.tiktok.likes",
                "system",
                "likes.baseline.complete",
                format!("pages={pages_read}\nitems={}", items.len()),
            );
        }
        let mut result = download_liked_videos(
            &session,
            &items,
            &existing,
            &disk_media_by_id,
            &mut report_progress,
            &is_cancelled,
        )?;
        result.pages_read = pages_read;
        result.stopped_incrementally = stopped_incrementally;
        Ok(result)
    })();
    if let Err(error) = &result {
        connector_debug::append_current(
            "internal.tiktok.likes",
            "error",
            "likes.sync.failed",
            error.clone(),
        );
    }
    result
}

fn create_likes_window(
    app: &AppHandle,
    session: &StoredSession,
) -> Result<WebviewWindow<Wry>, String> {
    let label = format!("{LIKES_WINDOW_LABEL_PREFIX}-{}", session.account_id);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.close();
    }
    let account_name = session.account_name.clone();
    let user_agent = session
        .user_agent
        .clone()
        .filter(|value| !value.trim().is_empty());
    let cookies = session.cookies.clone();
    let app_handle = app.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| {
            let url = TIKTOK_HOME
                .parse()
                .map_err(|error| format!("Invalid TikTok URL: {error}"))?;
            let mut builder =
                WebviewWindowBuilder::new(&app_handle, &label, WebviewUrl::External(url))
                    .title(format!("TikTok liked videos — {account_name}"))
                    .visible(false)
                    .skip_taskbar(true)
                    .incognito(true)
                    .initialization_script(
                        r#"
delete Object.getPrototypeOf(navigator).webdriver;
Object.defineProperty(Document.prototype, "hidden", { get: () => false });
Object.defineProperty(Document.prototype, "visibilityState", { get: () => "visible" });
(() => {
  const silence = (media) => {
    try {
      media.muted = true;
      media.volume = 0;
      media.pause();
    } catch {}
  };
  HTMLMediaElement.prototype.play = function () {
    silence(this);
    return Promise.resolve();
  };
  Object.defineProperty(HTMLMediaElement.prototype, "muted", {
    configurable: true,
    get() { return true; },
    set() {}
  });
  Object.defineProperty(HTMLMediaElement.prototype, "autoplay", {
    configurable: true,
    get() { return false; },
    set() {}
  });
  const silenceAll = () =>
    document.querySelectorAll("audio,video").forEach(silence);
  const observe = () => {
    silenceAll();
    if (document.documentElement) {
      new MutationObserver(silenceAll).observe(document.documentElement, {
        childList: true,
        subtree: true
      });
    }
  };
  document.addEventListener("DOMContentLoaded", observe, { once: true });
  observe();
})();
"#,
                    );
            if let Some(user_agent) = user_agent.as_deref() {
                builder = builder.user_agent(user_agent);
            }
            let window = builder
                .build()
                .map_err(|error| format!("Could not create the TikTok WebView: {error}"))?;
            inject_cookies(&window, &cookies)?;
            window
                .navigate(
                    TIKTOK_HOME
                        .parse()
                        .map_err(|error| format!("Invalid TikTok URL: {error}"))?,
                )
                .map_err(|error| format!("Could not navigate the TikTok WebView: {error}"))?;
            Ok(window)
        })();
        let _ = sender.send(result);
    })
    .map_err(|error| format!("Could not schedule TikTok WebView creation: {error}"))?;
    receiver
        .recv_timeout(WINDOW_CREATION_TIMEOUT)
        .map_err(|_| "Timed out while creating the TikTok WebView.".to_string())?
}

fn load_tiktok_session(request: &TikTokLikesSourceRequest) -> Result<StoredSession, String> {
    let mut layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    if let Some(media_root) = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'storage.media_root' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .filter(|value| !value.trim().is_empty())
    {
        layout.media_root = PathBuf::from(media_root);
    }
    let row = load_tiktok_account_session(&connection, &request.account_id)?;
    let yt_dlp = connector_runtime::resolve_connector_executable(&connection, &layout, "yt-dlp")?;
    let output_root = request.profile_root.join("Liked");
    let legacy_output_roots = vec![request.profile_root.join("Likes")];
    let cookie_root = layout
        .cache_root
        .join("tiktok-likes")
        .join(&request.source_id);
    drop(connection);

    let payload = session_secret_store::load_secret(&layout, &row.2)?;
    let (metadata, cookies) =
        if let Ok(envelope) = serde_json::from_str::<StoredSessionEnvelope>(&payload) {
            (envelope.metadata, envelope.cookies)
        } else {
            let cookies = serde_json::from_str::<Vec<StoredCookie>>(&payload)
                .map_err(|_| "Stored TikTok session has an unsupported JSON shape.".to_string())?;
            (StoredSessionMetadata::default(), cookies)
        };
    if cookies.is_empty() {
        return Err("Stored TikTok session does not contain any cookies.".to_string());
    }

    Ok(StoredSession {
        account_id: row.0,
        account_name: row.1,
        source_id: request.source_id.clone(),
        source_handle: request.source_handle.clone(),
        profile_root: request.profile_root.clone(),
        user_agent: metadata.user_agent,
        cookies,
        output_root,
        legacy_output_roots,
        cookie_root,
        yt_dlp,
    })
}

fn load_tiktok_account_session(
    connection: &rusqlite::Connection,
    account_id: &str,
) -> Result<(String, String, String), String> {
    connection
        .query_row(
            "SELECT a.id, a.display_name, s.secret_ref
             FROM provider_accounts a
             JOIN provider_account_sessions s ON s.account_id = a.id
             WHERE a.id = ?1
               AND lower(a.provider) = 'tiktok'
             LIMIT 1",
            [account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "TikTok account '{}' does not exist or has no stored session.",
                account_id
            )
        })
}

fn inject_cookies(window: &WebviewWindow<Wry>, cookies: &[StoredCookie]) -> Result<(), String> {
    for stored in cookies {
        if stored.domain.trim().is_empty() || stored.name.trim().is_empty() {
            continue;
        }
        // TikTokApi's proven session path flattens every imported cookie onto
        // the initial host. Keep the same behavior here instead of relying on
        // WebView2's interpretation of leading-dot cookie domains.
        let cookie = Cookie::build((stored.name.clone(), stored.value.clone()))
            .domain("www.tiktok.com")
            .path(stored.path.clone())
            .secure(stored.secure)
            .http_only(stored.http_only)
            .build();
        window.set_cookie(cookie).map_err(|error| {
            format!(
                "Could not inject TikTok cookie '{}' for domain '{}': {error}",
                stored.name, "www.tiktok.com"
            )
        })?;
    }
    Ok(())
}

fn run_probe<F, C>(
    window: &WebviewWindow<Wry>,
    ms_token: &str,
    item_limit: usize,
    known_present_ids: &HashSet<String>,
    incremental_active: bool,
    known_page_threshold: usize,
    report_progress: &mut F,
    is_cancelled: &C,
) -> Result<Value, String>
where
    F: FnMut(Option<u32>, String, String, bool, Option<u32>),
    C: Fn() -> bool,
{
    if let Err(error) = wait_until(
        window,
        "Boolean(window.byted_acrawler?.frontierSign)",
        PAGE_READY_TIMEOUT,
        |value| value.as_bool().unwrap_or(false),
        "TikTok did not expose window.byted_acrawler.frontierSign in WebView2.",
        is_cancelled,
    ) {
        let state = evaluate_json(
            window,
            r#"({
  href: location.href,
  title: document.title,
  readyState: document.readyState,
  visibilityState: document.visibilityState,
  webdriver: navigator.webdriver ?? null,
  userAgent: navigator.userAgent,
  bodyStart: (document.body?.innerText ?? "").slice(0, 160)
})"#,
        )
        .unwrap_or_else(|diagnostic_error| json!({ "diagnosticError": diagnostic_error }));
        return Err(format!("{error} Page state: {state}"));
    }
    if let Ok(url) = TIKTOK_HOME.parse() {
        if let Ok(cookies) = window.cookies_for_url(url) {
            let has_session = cookies.iter().any(|cookie| {
                matches!(
                    cookie.name(),
                    "sessionid" | "sessionid_ss" | "sid_tt" | "sid_guard"
                )
            });
            let has_ms_token = cookies
                .iter()
                .any(|cookie| cookie.name().eq_ignore_ascii_case("msToken"));
            if !has_session || !has_ms_token {
                return Err("TikTok WebView did not retain the authenticated cookies.".to_string());
            }
        }
    }

    let token_json = serde_json::to_string(ms_token).map_err(|error| error.to_string())?;
    let known_ids_json =
        serde_json::to_string(known_present_ids).map_err(|error| error.to_string())?;
    let script = build_probe_script(
        &token_json,
        item_limit,
        &known_ids_json,
        incremental_active,
        known_page_threshold,
    );
    window
        .eval(script)
        .map_err(|error| format!("Could not start the TikTok likes request: {error}"))?;

    let started = Instant::now();
    let mut last_activity = started;
    let mut last_progress = String::new();
    loop {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }
        let now = Instant::now();
        if let Some(error) = likes_request_timeout_reason(started, last_activity, now) {
            return Err(error);
        }
        let result = evaluate_json(window, "window.__ninjaCrawlerTikTokLikesResult ?? null")?;
        if !result.is_null() {
            return Ok(result);
        }
        let progress = evaluate_json(window, "window.__ninjaCrawlerTikTokLikesProgress ?? null")?;
        if !progress.is_null() {
            let serialized = progress.to_string();
            if serialized != last_progress {
                last_progress = serialized;
                last_activity = Instant::now();
                let pages = progress.get("pages").and_then(Value::as_u64).unwrap_or(0);
                let items = progress.get("items").and_then(Value::as_u64).unwrap_or(0);
                let consecutive_known_pages = progress
                    .get("consecutiveKnownPages")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                report_progress(
                    None,
                    "Listing liked videos".to_string(),
                    format!("Read {pages} page(s); found {items} video(s)."),
                    true,
                    Some(0),
                );
                connector_debug::append_current(
                    "internal.tiktok.likes",
                    "response",
                    "likes.page",
                    format!(
                        "pages={pages}\nitems={items}\nconsecutive_known_pages={consecutive_known_pages}"
                    ),
                );
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn likes_request_timeout_reason(
    started: Instant,
    last_activity: Instant,
    now: Instant,
) -> Option<String> {
    if now.duration_since(started) >= REQUEST_MAX_DURATION {
        return Some("TikTok likes request exceeded the 6-hour safety limit.".to_string());
    }
    if now.duration_since(last_activity) >= REQUEST_INACTIVITY_TIMEOUT {
        return Some(format!(
            "TikTok likes request made no progress for {} seconds.",
            REQUEST_INACTIVITY_TIMEOUT.as_secs()
        ));
    }
    None
}

fn wait_until<C>(
    window: &WebviewWindow<Wry>,
    script: &str,
    timeout: Duration,
    predicate: impl Fn(&Value) -> bool,
    timeout_message: &str,
    is_cancelled: &C,
) -> Result<Value, String>
where
    C: Fn() -> bool,
{
    let started = Instant::now();
    while started.elapsed() < timeout {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }
        let value = evaluate_json(window, script)?;
        if predicate(&value) {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(timeout_message.to_string())
}

fn evaluate_json(window: &WebviewWindow<Wry>, script: &str) -> Result<Value, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .eval_with_callback(script, move |serialized| {
            let _ = sender.send(serialized);
        })
        .map_err(|error| format!("WebView2 JavaScript evaluation failed: {error}"))?;
    let serialized = receiver
        .recv_timeout(JAVASCRIPT_TIMEOUT)
        .map_err(|_| "WebView2 did not return a JavaScript result.".to_string())?;
    serde_json::from_str(&serialized)
        .map_err(|error| format!("WebView2 returned invalid JSON: {error}"))
}

fn build_probe_script(
    ms_token_json: &str,
    item_limit: usize,
    known_ids_json: &str,
    incremental_active: bool,
    known_page_threshold: usize,
) -> String {
    format!(
        r#"
window.__ninjaCrawlerTikTokLikesResult = null;
window.__ninjaCrawlerTikTokLikesProgress = {{ pages: 0, items: 0 }};
(async () => {{
  try {{
    const accountResponse = await fetch(
      "https://www.tiktok.com/passport/web/account/info/?aid=1459",
      {{ credentials: "include" }}
    );
    const accountBody = await accountResponse.json();
    const account = accountBody?.data ?? {{}};
    const username = account.username ?? account.unique_id ?? account.uniqueId ?? "";
    const userId = account.user_id ?? account.userId ?? "";
    const secUid = account.sec_user_id ?? account.secUid ?? "";
    if (!accountResponse.ok || !username || !secUid) {{
      throw new Error(
        `Authenticated identity unavailable (HTTP ${{accountResponse.status}}, message=${{accountBody?.message ?? "unknown"}}).`
      );
    }}

    const language = navigator.language || "en-US";
    const platform = navigator.platform || "Win32";
    const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const randomDigits = (length) =>
      Array.from({{ length }}, () => Math.floor(Math.random() * 10)).join("");
    const baseParams = {{
      aid: "1988",
      app_language: language,
      app_name: "tiktok_web",
      browser_language: language,
      browser_name: "Mozilla",
      browser_online: "true",
      browser_platform: platform,
      browser_version: navigator.userAgent,
      channel: "tiktok_web",
      cookie_enabled: "true",
      device_id: "7" + randomDigits(18),
      device_platform: "web_pc",
      focus_state: "true",
      from_page: "user",
      history_len: String(1 + Math.floor(Math.random() * 10)),
      is_fullscreen: "false",
      is_page_visible: "true",
      language,
      os: platform,
      priority_region: "",
      referer: "",
      region: "US",
      screen_height: String(600 + Math.floor(Math.random() * 481)),
      screen_width: String(800 + Math.floor(Math.random() * 1121)),
      tz_name: timezone,
      webcast_language: language,
      secUid,
      count: "30",
      msToken: {ms_token_json}
    }};
    const encode = (value) =>
      encodeURIComponent(String(value)).replace(/%3D/gi, "=");
    const requestedLimit = {item_limit};
    const effectiveLimit = requestedLimit > 0 ? Math.min(requestedLimit, 10000) : 10000;
    const incrementalActive = {incremental_active};
    const knownPageThreshold = {known_page_threshold};
    const knownIds = new Set({known_ids_json});
    const items = [];
    const seenIds = new Set();
    let cursor = "0";
    let hasMore = true;
    let pages = 0;
    let httpStatus = 200;
    let statusCode = 0;
    let statusMessage = "";
    let truncated = false;
    let stoppedIncrementally = false;
    let consecutiveKnownPages = 0;

    while (hasMore && items.length < effectiveLimit) {{
      const params = {{ ...baseParams, cursor }};
      const query = Object.entries(params)
        .map(([key, value]) => `${{encode(key)}}=${{encode(value)}}`)
        .join("&");
      const unsignedUrl = `https://www.tiktok.com/api/favorite/item_list?${{query}}`;
      const signature = window.byted_acrawler.frontierSign(unsignedUrl);
      const xBogus = signature?.["X-Bogus"];
      if (!xBogus) {{
        throw new Error("frontierSign did not return X-Bogus.");
      }}

      const likesResponse = await fetch(`${{unsignedUrl}}&X-Bogus=${{xBogus}}`, {{
        credentials: "include"
      }});
      httpStatus = likesResponse.status;
      const likesText = await likesResponse.text();
      let likesBody;
      try {{
        likesBody = JSON.parse(likesText);
      }} catch {{
        throw new Error(`Likes endpoint returned non-JSON content (HTTP ${{likesResponse.status}}).`);
      }}
      statusCode = likesBody?.status_code ?? null;
      statusMessage = likesBody?.status_msg ?? "";
      if (!likesResponse.ok || statusCode !== 0) {{
        throw new Error(
          `Likes endpoint failed (HTTP ${{httpStatus}}, status=${{statusCode}}, message=${{statusMessage}}).`
        );
      }}
      const pageItems = Array.isArray(likesBody?.itemList) ? likesBody.itemList : [];
      const pageVideoIds = pageItems
        .filter((item) => item?.id && item?.author?.uniqueId && !item?.imagePost)
        .map((item) => String(item.id));
      for (const item of pageItems) {{
        if (
          items.length >= effectiveLimit
          || !item?.id
          || !item?.author?.uniqueId
          || item?.imagePost
          || seenIds.has(String(item.id))
        ) {{
          continue;
        }}
        seenIds.add(String(item.id));
        items.push({{
          id: String(item.id),
          author: item.author.uniqueId,
          url: `https://www.tiktok.com/@${{item.author.uniqueId}}/video/${{item.id}}`
        }});
      }}
      if (
        incrementalActive
        && pageVideoIds.length > 0
        && pageVideoIds.every((id) => knownIds.has(id))
      ) {{
        consecutiveKnownPages += 1;
      }} else {{
        consecutiveKnownPages = 0;
      }}
      pages += 1;
      window.__ninjaCrawlerTikTokLikesProgress = {{
        pages,
        items: items.length,
        consecutiveKnownPages
      }};
      const nextCursor = String(likesBody?.cursor ?? "");
      hasMore = Boolean(likesBody?.hasMore) && nextCursor !== "" && nextCursor !== cursor;
      cursor = nextCursor;
      if (
        incrementalActive
        && hasMore
        && consecutiveKnownPages >= knownPageThreshold
      ) {{
        stoppedIncrementally = true;
        break;
      }}
      if (pages >= 334 && hasMore) {{
        truncated = true;
        break;
      }}
      if (hasMore) {{
        await new Promise((resolve) => setTimeout(resolve, 250));
      }}
    }}
    if (requestedLimit === 0 && items.length >= effectiveLimit && hasMore) {{
      truncated = true;
    }}
    window.__ninjaCrawlerTikTokLikesResult = {{
      ok: true,
      identity: {{ username, userId, secUid }},
      httpStatus,
      statusCode,
      statusMessage,
      hasMore,
      cursor,
      pages,
      truncated,
      stoppedIncrementally,
      items
    }};
  }} catch (error) {{
    window.__ninjaCrawlerTikTokLikesResult = {{
      ok: false,
      error: String(error?.message ?? error)
    }};
  }}
}})();
"#
    )
}

fn download_liked_videos<F, C>(
    session: &StoredSession,
    items: &[LikedVideo],
    ledger: &HashMap<String, String>,
    disk_media_by_id: &HashMap<String, PathBuf>,
    report_progress: &mut F,
    is_cancelled: &C,
) -> Result<TikTokLikesSyncResult, String>
where
    F: FnMut(Option<u32>, String, String, bool, Option<u32>),
    C: Fn() -> bool,
{
    let mut stats = TikTokLikesSyncResult {
        discovered: items.len(),
        ..TikTokLikesSyncResult::default()
    };
    let cookie_path = session
        .cookie_root
        .join(format!(".ninjacrawler-cookies-{}.txt", Uuid::new_v4()));
    write_netscape_cookies(&cookie_path, &session.cookies)?;

    let result = (|| {
        for (index, item) in items.iter().enumerate() {
            if is_cancelled() {
                return Err("source sync cancelled by user".to_string());
            }
            let position = index + 1;
            let percent = if items.is_empty() {
                100
            } else {
                ((position as f64 / items.len() as f64) * 100.0).round() as u32
            };
            report_progress(
                Some(percent.min(100)),
                "Downloading liked videos".to_string(),
                format!("Video {position} of {}", items.len()),
                false,
                Some(stats.downloaded as u32),
            );
            let ledger_path = ledger
                .get(&item.id)
                .and_then(|relative| resolve_stored_media(session, relative));
            let id_path = disk_media_by_id.get(&item.id).cloned();
            if let Some(existing_path) = id_path.clone().or_else(|| ledger_path.clone()) {
                let repaired_ledger = id_path
                    .as_ref()
                    .is_some_and(|path| ledger_path.as_ref() != Some(path));
                if repaired_ledger {
                    persist_download(session, item, &existing_path)?;
                }
                stats.skipped_existing += 1;
                if stats.skipped_existing == 1 || stats.skipped_existing % 50 == 0 {
                    connector_debug::append_current(
                        "internal.tiktok.likes",
                        "system",
                        "likes.media.skip",
                        format!(
                            "item_id={}\nposition={position}\ntotal={}\nskipped_existing={}\nmatch={}\npath={}\nledger_repaired={repaired_ledger}",
                            item.id,
                            items.len(),
                            stats.skipped_existing,
                            if id_path.is_some() {
                                "provider_item_id"
                            } else {
                                "ledger_path"
                            },
                            existing_path.display(),
                        ),
                    );
                }
                continue;
            }
            match download_liked_video(session, item, &cookie_path) {
                Ok(path) => {
                    persist_download(session, item, &path)?;
                    stats.downloaded += 1;
                    connector_debug::append_current(
                        "internal.tiktok.likes",
                        "response",
                        "likes.media.downloaded",
                        format!("item_id={}\npath={}", item.id, path.display()),
                    );
                }
                Err(error) => {
                    stats.failed += 1;
                    stats.failures.push(format!("{}: {}", item.id, error));
                    connector_debug::append_current(
                        "internal.tiktok.likes",
                        "error",
                        "likes.media.failed",
                        format!("item_id={}\nerror={error}", item.id),
                    );
                }
            }
        }
        report_progress(
            Some(100),
            "Liked videos complete".to_string(),
            format!(
                "Downloaded {}, skipped {} existing, failed {}.",
                stats.downloaded, stats.skipped_existing, stats.failed
            ),
            false,
            Some(stats.downloaded as u32),
        );
        connector_debug::append_current(
            "internal.tiktok.likes",
            "system",
            "likes.sync.complete",
            format!(
                "discovered={}\ndownloaded={}\nskipped_existing={}\nfailed={}",
                stats.discovered, stats.downloaded, stats.skipped_existing, stats.failed
            ),
        );
        Ok(stats)
    })();
    let _ = fs::remove_file(cookie_path);
    result
}

fn load_existing_ledger(account_id: &str) -> Result<HashMap<String, String>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT provider_item_key, relative_path
             FROM account_sync_media_ledger
             WHERE provider = 'tiktok'
               AND account_id = ?1
               AND sync_scope = 'liked_videos'",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut ledger = HashMap::new();
    for row in rows {
        let (key, relative_path) = row.map_err(|error| error.to_string())?;
        ledger.insert(key, relative_path);
    }
    Ok(ledger)
}

fn resolve_stored_media(session: &StoredSession, relative_path: &str) -> Option<PathBuf> {
    std::iter::once(&session.output_root)
        .chain(session.legacy_output_roots.iter())
        .map(|root| root.join(relative_path))
        .find(|path| path.is_file())
}

fn load_liked_media_index(session: &StoredSession) -> Result<HashMap<String, PathBuf>, String> {
    let roots = std::iter::once(session.output_root.clone())
        .chain(session.legacy_output_roots.iter().cloned())
        .collect::<Vec<_>>();
    index_media_by_tiktok_id(&roots)
}

fn index_media_by_tiktok_id(roots: &[PathBuf]) -> Result<HashMap<String, PathBuf>, String> {
    let mut media_by_id = HashMap::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if !is_liked_video_file(&path) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            for item_id in numeric_id_tokens(name) {
                media_by_id.entry(item_id).or_insert_with(|| path.clone());
            }
        }
    }
    Ok(media_by_id)
}

fn is_liked_video_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "mp4" | "mkv" | "webm" | "mov" | "m4v"
                )
            })
}

fn numeric_id_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| (15..=22).contains(&token.len()))
        .map(str::to_string)
        .collect()
}

fn has_completed_full_scan(account_id: &str, source_id: &str) -> Result<bool, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    connection
        .query_row(
            "SELECT full_scan_completed_at IS NOT NULL
             FROM account_sync_scope_state
             WHERE provider = 'tiktok'
               AND account_id = ?1
               AND source_id = ?2
               AND sync_scope = 'liked_videos'
             LIMIT 1",
            [account_id, source_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(|error| error.to_string())
}

fn mark_full_scan_completed(account_id: &str, source_id: &str) -> Result<(), String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    let now = Utc::now().to_rfc3339();
    connection
        .execute(
            "INSERT INTO account_sync_scope_state (
                provider, account_id, source_id, sync_scope,
                full_scan_completed_at, updated_at
             ) VALUES ('tiktok', ?1, ?2, 'liked_videos', ?3, ?3)
             ON CONFLICT(provider, account_id, source_id, sync_scope)
             DO UPDATE SET
                full_scan_completed_at = excluded.full_scan_completed_at,
                updated_at = excluded.updated_at",
            rusqlite::params![account_id, source_id, now],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn download_liked_video(
    session: &StoredSession,
    item: &LikedVideo,
    cookie_path: &Path,
) -> Result<PathBuf, String> {
    let output_template = session
        .output_root
        .join("%(uploader,uploader_id,id)s_%(id)s.%(ext)s")
        .to_string_lossy()
        .to_string();
    let mut command = Command::new(&session.yt_dlp);
    configure_background_command(&mut command);
    connector_debug::append_current(
        "yt-dlp",
        "call",
        "likes.media.download",
        format!(
            "item_id={}\nurl={}\noutput_root={}",
            item.id,
            item.url,
            session.output_root.display()
        ),
    );
    let output = command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .arg("--no-playlist")
        .arg("--no-warnings")
        .arg("--no-cookies-from-browser")
        .arg("--no-mtime")
        .arg("--impersonate")
        .arg("chrome")
        .arg("--cookies")
        .arg(cookie_path)
        .arg("--print")
        .arg("after_move:%(filepath)s")
        .arg("-o")
        .arg(output_template)
        .arg(&item.url)
        .output()
        .map_err(|error| format!("Failed to run yt-dlp: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout
        .lines()
        .rev()
        .map(str::trim)
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("yt-dlp did not report an output file.");
            format!("yt-dlp exited with {:?}: {detail}", output.status.code())
        })?;
    validate_video_file(&path)?;
    Ok(path)
}

fn persist_download(session: &StoredSession, item: &LikedVideo, path: &Path) -> Result<(), String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    let now = Utc::now().to_rfc3339();
    let relative_path = path
        .strip_prefix(&session.output_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    connection
        .execute(
            "INSERT INTO account_sync_media_ledger (
                provider, account_id, sync_scope, provider_item_key, source_handle,
                source_url, relative_path, media_type, first_seen_at, last_seen_at
             ) VALUES ('tiktok', ?1, 'liked_videos', ?2, ?3, ?4, ?5, 'video', ?6, ?6)
             ON CONFLICT(provider, account_id, sync_scope, provider_item_key)
             DO UPDATE SET
                source_handle = excluded.source_handle,
                source_url = excluded.source_url,
                relative_path = excluded.relative_path,
                last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                &session.account_id,
                &item.id,
                &item.author,
                &item.url,
                &relative_path,
                &now
            ],
        )
        .map_err(|error| error.to_string())?;
    let profile_relative_path = path
        .strip_prefix(&session.profile_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let media_ledger_key = format!("liked_{}", item.id);
    connection
        .execute(
            "INSERT INTO provider_sync_post_ledger (
                provider, source_id, account_id, source_handle, provider_post_key,
                provider_post_code, media_section, first_seen_at, last_seen_at
             ) VALUES ('tiktok', ?1, ?2, ?3, ?4, '', 'likes', ?5, ?5)
             ON CONFLICT(provider, source_id, provider_post_key)
             DO UPDATE SET
                account_id = excluded.account_id,
                source_handle = excluded.source_handle,
                media_section = excluded.media_section,
                last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                &session.source_id,
                &session.account_id,
                &session.source_handle,
                &item.id,
                &now
            ],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO provider_sync_media_ledger (
                provider, source_id, account_id, source_handle, provider_media_key,
                media_type, media_section, relative_path, first_seen_at, last_seen_at
             ) VALUES ('tiktok', ?1, ?2, ?3, ?4, 'video', 'likes', ?5, ?6, ?6)
             ON CONFLICT(provider, source_id, provider_media_key, media_type)
             DO UPDATE SET
                account_id = excluded.account_id,
                source_handle = excluded.source_handle,
                media_section = excluded.media_section,
                relative_path = excluded.relative_path,
                last_seen_at = excluded.last_seen_at",
            rusqlite::params![
                &session.source_id,
                &session.account_id,
                &session.source_handle,
                &media_ledger_key,
                &profile_relative_path,
                &now
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn write_netscape_cookies(path: &Path, cookies: &[StoredCookie]) -> Result<(), String> {
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
    for cookie in cookies {
        let domain = if cookie.http_only {
            format!("#HttpOnly_{}", cookie.domain)
        } else {
            cookie.domain.clone()
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t0\t{}\t{}",
            domain,
            if cookie.domain.starts_with('.') {
                "TRUE"
            } else {
                "FALSE"
            },
            cookie.path,
            if cookie.secure { "TRUE" } else { "FALSE" },
            cookie.name,
            cookie.value
        ));
    }
    fs::write(path, lines.join("\n")).map_err(|error| error.to_string())
}

fn cleanup_stale_cookie_files(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_runtime_cookie = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with(".ninjacrawler-cookies-") && name.ends_with(".txt"))
            .unwrap_or(false);
        if is_runtime_cookie && path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

fn validate_video_file(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() < 10_000 {
        return Err(format!(
            "The downloaded result is too small to be a video ({} bytes).",
            metadata.len()
        ));
    }
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut header = [0_u8; 32];
    let read = file.read(&mut header).map_err(|error| error.to_string())?;
    let looks_like_mp4 = header[..read].windows(4).any(|bytes| bytes == b"ftyp");
    let looks_like_webm = header[..read].starts_with(&[0x1a, 0x45, 0xdf, 0xa3]);
    if !looks_like_mp4 && !looks_like_webm {
        return Err("The downloaded result is not a recognized video file.".to_string());
    }
    Ok(())
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn default_cookie_path() -> String {
    "/".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_tiktok_session_remains_available_when_account_is_degraded() {
        let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
        connection
            .execute_batch(
                "CREATE TABLE provider_accounts (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    display_name TEXT NOT NULL,
                    auth_state TEXT NOT NULL
                 );
                 CREATE TABLE provider_account_sessions (
                    account_id TEXT PRIMARY KEY,
                    secret_ref TEXT NOT NULL
                 );
                 INSERT INTO provider_accounts (id, provider, display_name, auth_state)
                 VALUES ('account-1', 'tiktok', 'gui', 'degraded');
                 INSERT INTO provider_account_sessions (account_id, secret_ref)
                 VALUES ('account-1', 'account-1');",
            )
            .expect("account fixture");

        let session =
            load_tiktok_account_session(&connection, "account-1").expect("stored session");

        assert_eq!(
            session,
            (
                "account-1".to_string(),
                "gui".to_string(),
                "account-1".to_string()
            )
        );
    }

    #[test]
    fn incremental_probe_receives_known_ids_and_page_threshold() {
        let script = build_probe_script("\"token\"", 0, "[\"video-1\",\"video-2\"]", true, 4);

        assert!(script.contains("const incrementalActive = true;"));
        assert!(script.contains("const knownPageThreshold = 4;"));
        assert!(script.contains("new Set([\"video-1\",\"video-2\"])"));
        assert!(script.contains("stoppedIncrementally = true;"));
    }

    #[test]
    fn disk_index_uses_tiktok_id_independently_from_handle_and_file_name() {
        let temp = tempfile::tempdir().expect("temp directory");
        let liked = temp.path().join("Liked");
        fs::create_dir_all(&liked).expect("liked directory");
        let old_handle = liked.join("old_handle_1743950164_7490208900055190789.mp4");
        fs::write(&old_handle, "video").expect("video fixture");
        fs::write(
            liked.join("new_handle_7490208900055190790.mp4.part"),
            "partial",
        )
        .expect("partial download fixture");

        let index = index_media_by_tiktok_id(&[liked]).expect("disk index");

        assert_eq!(
            index.get("7490208900055190789"),
            Some(&old_handle),
            "the stable TikTok ID must win even when the handle-based name changes"
        );
        assert!(!index.contains_key("1743950164"));
        assert!(!index.contains_key("7490208900055190790"));
    }

    #[test]
    fn likes_request_watchdog_resets_when_pagination_keeps_progressing() {
        let started = Instant::now();
        let after_old_absolute_timeout = started + Duration::from_secs(301);
        let recent_progress = after_old_absolute_timeout - Duration::from_secs(1);

        assert_eq!(
            likes_request_timeout_reason(started, recent_progress, after_old_absolute_timeout),
            None
        );
    }

    #[test]
    fn likes_request_watchdog_distinguishes_inactivity_from_safety_limit() {
        let started = Instant::now();
        assert_eq!(
            likes_request_timeout_reason(started, started, started + REQUEST_INACTIVITY_TIMEOUT)
                .as_deref(),
            Some("TikTok likes request made no progress for 180 seconds.")
        );

        let safety_limit = started + REQUEST_MAX_DURATION;
        assert_eq!(
            likes_request_timeout_reason(started, safety_limit, safety_limit).as_deref(),
            Some("TikTok likes request exceeded the 6-hour safety limit.")
        );
    }

    #[test]
    fn stale_cookie_cleanup_preserves_unrelated_files() {
        let temp = tempfile::tempdir().expect("temp directory");
        let stale = temp.path().join(".ninjacrawler-cookies-old.txt");
        let unrelated = temp.path().join("keep.txt");
        fs::write(&stale, "secret").expect("stale cookie");
        fs::write(&unrelated, "keep").expect("unrelated file");

        cleanup_stale_cookie_files(temp.path());

        assert!(!stale.exists());
        assert!(unrelated.exists());
    }
}
