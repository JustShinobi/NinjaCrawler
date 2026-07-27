use crate::infrastructure::media_tool_runtime;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// Rich media metadata harvested from `ffprobe`, surfaced next to duplicate-scan
/// results (VDF-inspired). All fields are best-effort: `None` means "not probed
/// yet", "ffprobe unavailable", or "no such stream" — the UI renders a `–`
/// placeholder in every case, never an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProbedMediaMetadata {
    pub bitrate_kbps: Option<u64>,
    pub video_codec: Option<String>,
    pub frame_rate: Option<f64>,
    pub audio_summary: Option<String>,
}

impl ProbedMediaMetadata {
    fn is_empty(&self) -> bool {
        self.bitrate_kbps.is_none()
            && self.video_codec.is_none()
            && self.frame_rate.is_none()
            && self.audio_summary.is_none()
    }
}

fn cache() -> &'static Mutex<HashMap<String, (i64, ProbedMediaMetadata)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (i64, ProbedMediaMetadata)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns metadata already cached for this exact file (matched by mtime), or
/// `None` when nothing has been probed yet — never touches ffprobe itself, so
/// it's safe to call on every status poll.
pub(crate) fn resolve_cached(path: &Path) -> Option<ProbedMediaMetadata> {
    let modified_at_ms = file_modified_at_ms(path)?;
    let key = cache_key(path);
    let cache = cache().lock().ok()?;
    cache
        .get(&key)
        .filter(|(cached_mtime, _)| *cached_mtime == modified_at_ms)
        .map(|(_, metadata)| metadata.clone())
}

/// Runs ffprobe against `path` (when the cache is stale or empty for its current
/// mtime) and caches the result. Meant to run off the calling thread, in a
/// background pass — mirrors `generate_media_thumbnail`'s "idempotent, never
/// fails the caller" contract: a missing ffprobe or an unreadable file is simply
/// skipped, leaving the fields `None`.
pub(crate) fn probe_and_cache(path: &Path) {
    let Some(modified_at_ms) = file_modified_at_ms(path) else {
        return;
    };
    let key = cache_key(path);
    if let Ok(cache) = cache().lock() {
        if cache
            .get(&key)
            .is_some_and(|(mtime, _)| *mtime == modified_at_ms)
        {
            return;
        }
    }
    let Some(metadata) = probe_now(path) else {
        return;
    };
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, (modified_at_ms, metadata));
    }
}

fn probe_now(path: &Path) -> Option<ProbedMediaMetadata> {
    let ffprobe = media_tool_runtime::ffprobe_executable()?;
    let mut command = Command::new(&ffprobe);
    media_tool_runtime::configure_tool_path(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=bit_rate:stream=codec_type,codec_name,avg_frame_rate,r_frame_rate,channels,channel_layout",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let metadata = parse_ffprobe_json(&stdout);
    (!metadata.is_empty()).then_some(metadata)
}

/// Pure parser for ffprobe's `-of json` output — kept separate from process
/// spawning so it's testable with fixtures. Tolerant of missing/malformed
/// fields: anything it can't find comes back as `None`.
pub(crate) fn parse_ffprobe_json(json: &str) -> ProbedMediaMetadata {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return ProbedMediaMetadata::default();
    };

    let bitrate_kbps = value
        .get("format")
        .and_then(|format| format.get("bit_rate"))
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|bits_per_second| bits_per_second / 1000);

    let streams = value
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let video_stream = streams
        .iter()
        .find(|stream| stream_codec_type(stream) == Some("video"));
    let audio_stream = streams
        .iter()
        .find(|stream| stream_codec_type(stream) == Some("audio"));

    let video_codec = video_stream
        .and_then(|stream| stream.get("codec_name"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let frame_rate = video_stream.and_then(|stream| {
        stream
            .get("avg_frame_rate")
            .and_then(|value| value.as_str())
            .filter(|value| *value != "0/0")
            .or_else(|| stream.get("r_frame_rate").and_then(|value| value.as_str()))
            .and_then(parse_frame_rate_fraction)
    });
    let audio_summary = Some(match audio_stream {
        Some(stream) => {
            let codec = stream
                .get("codec_name")
                .and_then(|value| value.as_str())
                .unwrap_or("audio");
            match audio_channel_label(stream) {
                Some(label) => format!("{codec} ({label})"),
                None => codec.to_string(),
            }
        }
        None => "No audio".to_string(),
    });

    ProbedMediaMetadata {
        bitrate_kbps,
        video_codec,
        frame_rate,
        audio_summary,
    }
}

fn stream_codec_type(stream: &serde_json::Value) -> Option<&str> {
    stream.get("codec_type").and_then(serde_json::Value::as_str)
}

fn audio_channel_label(stream: &serde_json::Value) -> Option<String> {
    stream
        .get("channel_layout")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && *value != "unknown")
        .map(str::to_string)
        .or_else(|| {
            stream
                .get("channels")
                .and_then(serde_json::Value::as_u64)
                .map(|channels| match channels {
                    1 => "mono".to_string(),
                    2 => "stereo".to_string(),
                    other => format!("{other}ch"),
                })
        })
}

fn parse_frame_rate_fraction(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    if denominator <= 0.0 {
        return None;
    }
    let fps = numerator / denominator;
    (fps.is_finite() && fps > 0.0).then(|| (fps * 100.0).round() / 100.0)
}

fn cache_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn file_modified_at_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bitrate_codec_frame_rate_and_audio() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "avg_frame_rate": "30000/1001", "r_frame_rate": "30000/1001"},
                {"codec_type": "audio", "codec_name": "aac", "channels": 2, "channel_layout": "stereo"}
            ],
            "format": {"bit_rate": "3512000"}
        }"#;
        let metadata = parse_ffprobe_json(json);
        assert_eq!(metadata.bitrate_kbps, Some(3512));
        assert_eq!(metadata.video_codec.as_deref(), Some("h264"));
        assert_eq!(metadata.frame_rate, Some(29.97));
        assert_eq!(metadata.audio_summary.as_deref(), Some("aac (stereo)"));
    }

    #[test]
    fn falls_back_to_r_frame_rate_when_avg_is_zero() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "vp9", "avg_frame_rate": "0/0", "r_frame_rate": "25/1"}
            ],
            "format": {}
        }"#;
        let metadata = parse_ffprobe_json(json);
        assert_eq!(metadata.frame_rate, Some(25.0));
    }

    #[test]
    fn reports_no_audio_when_no_audio_stream_present() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "avg_frame_rate": "24/1"}
            ],
            "format": {"bit_rate": "1000000"}
        }"#;
        let metadata = parse_ffprobe_json(json);
        assert_eq!(metadata.audio_summary.as_deref(), Some("No audio"));
    }

    #[test]
    fn falls_back_to_channel_count_when_layout_is_missing() {
        let json = r#"{
            "streams": [
                {"codec_type": "audio", "codec_name": "mp3", "channels": 6}
            ]
        }"#;
        let metadata = parse_ffprobe_json(json);
        assert_eq!(metadata.audio_summary.as_deref(), Some("mp3 (6ch)"));
    }

    #[test]
    fn malformed_json_degrades_to_empty_metadata() {
        let metadata = parse_ffprobe_json("not json");
        assert_eq!(metadata, ProbedMediaMetadata::default());
        assert!(metadata.is_empty());
    }

    #[test]
    fn empty_streams_and_format_degrade_gracefully() {
        let metadata = parse_ffprobe_json(r#"{"streams": [], "format": {}}"#);
        assert_eq!(metadata.bitrate_kbps, None);
        assert_eq!(metadata.video_codec, None);
        assert_eq!(metadata.frame_rate, None);
        // No audio stream present, but the format still parses — this counts
        // as a successful (non-empty) probe result.
        assert_eq!(metadata.audio_summary.as_deref(), Some("No audio"));
        assert!(!metadata.is_empty());
    }

    #[test]
    fn cache_round_trips_by_path_and_mtime() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("clip.mp4");
        std::fs::write(&path, b"fake video bytes").expect("write");

        assert_eq!(resolve_cached(&path), None);

        let modified_at_ms = file_modified_at_ms(&path).expect("mtime");
        let metadata = ProbedMediaMetadata {
            bitrate_kbps: Some(1200),
            video_codec: Some("h264".to_string()),
            frame_rate: Some(30.0),
            audio_summary: Some("aac (stereo)".to_string()),
        };
        cache()
            .lock()
            .expect("cache lock")
            .insert(cache_key(&path), (modified_at_ms, metadata.clone()));

        assert_eq!(resolve_cached(&path), Some(metadata));
    }
}
