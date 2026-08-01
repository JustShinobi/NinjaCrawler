use super::*;

/// How many qualifying scans in a row must miss a post before it is called
/// removed. One is not enough: a listing can come back short for reasons that
/// look clean from here (a provider hiccup, a partially rendered page), and a
/// wrong "removed" badge is worse than a late one.
pub(crate) const UPSTREAM_MISSING_CONFIRMATIONS: i64 = 2;

/// Sections whose absence actually means "the author removed it".
///
/// Stories are excluded because they expire by design — every story ever
/// archived would be flagged within a day. TikTok likes are excluded because a
/// vanished entry means the profile owner un-liked it, not that the author
/// deleted anything. Saved/tagged collections belong to someone else's feed and
/// change for reasons that have nothing to do with the tracked profile.
fn section_reports_removal(section: &str) -> bool {
    let section = section.trim().to_ascii_lowercase();
    matches!(
        section.as_str(),
        "" | "timeline" | "feed" | "posts" | "reels" | "clips" | "videos" | "shorts" | "photos"
            | "media" | "reposts" | "gallery"
    )
}

/// Everything the caller must prove before absence is allowed to mean removal.
/// Each field maps to a way a listing can come back incomplete while the sync
/// itself reports success.
pub(crate) struct UpstreamScanQualification<'a> {
    pub(crate) provider: &'a str,
    pub(crate) source_id: &'a str,
    /// Sections the scan enumerated end to end.
    pub(crate) sections_scanned: &'a [String],
    /// Post keys observed in those sections, including ones skipped from
    /// download for already being on disk.
    pub(crate) observed_post_keys: &'a HashSet<String>,
    /// False when discovery stopped early (incremental stop still armed).
    pub(crate) enumerated_in_full: bool,
    /// True when a date window, `missing_only`, or any other filter narrowed
    /// the listing below the whole section.
    pub(crate) filtered: bool,
    /// True when the provider rate-limited, a section errored, or auth blocked
    /// part of the listing.
    pub(crate) truncated: bool,
}

impl UpstreamScanQualification<'_> {
    /// A scan only judges absence when it saw everything it was supposed to see.
    pub(crate) fn qualifies(&self) -> bool {
        self.enumerated_in_full
            && !self.filtered
            && !self.truncated
            && self.eligible_sections().next().is_some()
    }

    fn eligible_sections(&self) -> impl Iterator<Item = &String> {
        self.sections_scanned
            .iter()
            .filter(|section| section_reports_removal(section))
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct UpstreamPresenceOutcome {
    pub(crate) posts_seen: usize,
    /// Posts that crossed the confirmation threshold in this evaluation.
    pub(crate) flagged: usize,
    /// Posts that were missing before and showed up again.
    pub(crate) recovered: usize,
}

fn post_ledger_table(provider: &str) -> &'static str {
    if provider.eq_ignore_ascii_case("instagram") {
        "instagram_sync_post_ledger"
    } else {
        "provider_sync_post_ledger"
    }
}

/// The Instagram ledger predates the provider-neutral one and has no `provider`
/// column, so every statement here is scoped differently for it.
fn provider_scope_clause(provider: &str) -> &'static str {
    if provider.eq_ignore_ascii_case("instagram") {
        ""
    } else {
        " AND provider = ?provider"
    }
}

/// Compares what a qualifying scan saw against what the ledger holds, and moves
/// posts between `present` and `missing` accordingly.
///
/// Locally deleted media is skipped: `provider_deleted_media` records posts the
/// operator removed from disk on purpose, and the sync deliberately stops
/// re-downloading them. Their absence from a listing says nothing about the
/// provider.
pub(crate) fn evaluate_upstream_presence(
    connection: &Connection,
    qualification: &UpstreamScanQualification<'_>,
    timestamp: &str,
) -> Result<UpstreamPresenceOutcome, String> {
    if !qualification.qualifies() {
        return Ok(UpstreamPresenceOutcome::default());
    }

    let table = post_ledger_table(qualification.provider);
    let sections: Vec<String> = qualification
        .eligible_sections()
        .map(|section| section.trim().to_ascii_lowercase())
        .collect();
    let deleted_locally =
        load_provider_deleted_post_keys(connection, qualification.provider, qualification.source_id)
            .unwrap_or_default();

    let placeholders = sections
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let provider_clause = provider_scope_clause(qualification.provider).replace("?provider", "?1");
    let select = format!(
        "SELECT provider_post_key, upstream_state, missing_confirmations
         FROM {table}
         WHERE source_id = ?2{provider_clause}
           AND LOWER(TRIM(media_section)) IN ({placeholders})"
    );

    let mut rows: Vec<(String, String, i64)> = Vec::new();
    {
        let mut statement = connection
            .prepare(&select)
            .map_err(|error| error.to_string())?;
        let mut bindings: Vec<&dyn rusqlite::ToSql> =
            vec![&qualification.provider, &qualification.source_id];
        for section in &sections {
            bindings.push(section);
        }
        let mapped = statement
            .query_map(bindings.as_slice(), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|error| error.to_string())?;
        for row in mapped {
            rows.push(row.map_err(|error| error.to_string())?);
        }
    }

    let mut outcome = UpstreamPresenceOutcome {
        posts_seen: rows.len(),
        ..UpstreamPresenceOutcome::default()
    };
    let present_update = format!(
        "UPDATE {table}
         SET upstream_state = 'present', missing_confirmations = 0, missing_since = NULL,
             last_full_scan_at = ?3
         WHERE source_id = ?2{provider_clause} AND provider_post_key = ?4"
    );
    let absent_update = format!(
        "UPDATE {table}
         SET missing_confirmations = ?5,
             upstream_state = CASE WHEN ?5 >= ?6 THEN 'missing' ELSE upstream_state END,
             missing_since = CASE WHEN ?5 >= ?6 THEN COALESCE(missing_since, ?3) ELSE missing_since END,
             last_full_scan_at = ?3
         WHERE source_id = ?2{provider_clause} AND provider_post_key = ?4"
    );

    for (post_key, state, confirmations) in rows {
        if deleted_locally.contains(&post_key) {
            continue;
        }
        if qualification.observed_post_keys.contains(&post_key) {
            connection
                .execute(
                    &present_update,
                    params![
                        qualification.provider,
                        qualification.source_id,
                        timestamp,
                        post_key
                    ],
                )
                .map_err(|error| error.to_string())?;
            if state == "missing" {
                outcome.recovered += 1;
            }
            continue;
        }

        let next = confirmations + 1;
        connection
            .execute(
                &absent_update,
                params![
                    qualification.provider,
                    qualification.source_id,
                    timestamp,
                    post_key,
                    next,
                    UPSTREAM_MISSING_CONFIRMATIONS
                ],
            )
            .map_err(|error| error.to_string())?;
        if state != "missing" && next >= UPSTREAM_MISSING_CONFIRMATIONS {
            outcome.flagged += 1;
        }
    }

    project_upstream_state_onto_media_index(
        connection,
        qualification.provider,
        qualification.source_id,
        timestamp,
    )?;
    record_full_scan_run(connection, qualification, &sections, &outcome, timestamp)?;
    Ok(outcome)
}

/// Mirrors the ledger verdict onto the media index so the gallery, and later the
/// timeline and dashboard, can filter on it without joining two ledger shapes.
fn project_upstream_state_onto_media_index(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    let table = post_ledger_table(provider);
    let provider_clause = provider_scope_clause(provider).replace("?provider", "?1");
    let statement = format!(
        "UPDATE media_index
         SET upstream_state = COALESCE((
                 SELECT ledger.upstream_state FROM {table} ledger
                  WHERE ledger.source_id = media_index.source_id{provider_clause}
                    AND ledger.provider_post_key = media_index.provider_post_key
             ), media_index.upstream_state),
             updated_at = ?3
         WHERE source_id = ?2 AND provider_post_key IS NOT NULL"
    );
    connection
        .execute(&statement, params![provider, source_id, timestamp])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn record_full_scan_run(
    connection: &Connection,
    qualification: &UpstreamScanQualification<'_>,
    sections: &[String],
    outcome: &UpstreamPresenceOutcome,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO source_full_scan_runs (
                id, source_id, provider, sections, posts_seen, posts_flagged,
                posts_recovered, evaluated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                Uuid::new_v4().to_string(),
                qualification.source_id,
                qualification.provider,
                sections.join(","),
                outcome.posts_seen as i64,
                outcome.flagged as i64,
                outcome.recovered as i64,
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Wires the Instagram sync into the presence evaluation.
///
/// Instagram is the provider where this is easiest to get wrong: a normal sync
/// stops discovery as soon as it recognizes known posts, so only an explicit
/// `full_scan` produces a listing worth comparing against the ledger.
///
/// Best-effort by design — a presence evaluation must never fail a sync that
/// already downloaded media successfully.
pub(super) fn evaluate_instagram_upstream_presence(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &instagram_connector::InstagramConnectorRequest,
    result: &instagram_connector::InstagramConnectorResult,
    timestamp: &str,
) {
    let mut sections = Vec::new();
    if request.sections.timeline {
        sections.push("timeline".to_string());
    }
    if request.sections.reels {
        sections.push("reels".to_string());
    }

    let observed_post_keys = result
        .observed_posts
        .iter()
        .map(|post| normalize_instagram_post_ledger_key(&post.provider_post_key))
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();

    let qualification = UpstreamScanQualification {
        provider: "instagram",
        source_id: &context.source.id,
        sections_scanned: &sections,
        observed_post_keys: &observed_post_keys,
        enumerated_in_full: request.full_scan,
        filtered: request.missing_only
            || request.get_user_media_only
            || request.date_from_timestamp.is_some()
            || request.date_to_timestamp.is_some()
            || request.target_story_media_id.is_some(),
        truncated: result.rate_limited
            || !result.section_errors.is_empty()
            || !result.auth_disabled_sections.is_empty()
            || result.validation_error.is_some(),
    };

    if let Err(error) = evaluate_upstream_presence(connection, &qualification, timestamp) {
        let _ = runtime_log::append_workspace(
            "upstream_presence",
            "warn",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some("instagram"),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            "Failed to evaluate which posts are gone from the provider.",
            Some(error),
        );
    }
}

/// Wires the Twitter/X sync into the presence evaluation.
///
/// The equivalent of Instagram's `full_scan` here is the absence of an
/// incremental cutoff: a normal run passes `date.timestamp() >= cutoff or
/// abort()` to the parser, so the listing stops at the overlap point. A resume
/// cursor means this execution continued a previous partial pass, which is also
/// not a whole-section listing.
pub(super) fn evaluate_twitter_upstream_presence(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &twitter_connector::TwitterConnectorRequest,
    result: &twitter_connector::TwitterConnectorResult,
    timestamp: &str,
) {
    let mut sections = Vec::new();
    if request.models.media {
        sections.push("media".to_string());
    }
    if request.models.profile {
        sections.push("timeline".to_string());
    }

    let observed_post_keys = result
        .observed_posts
        .iter()
        .map(|post| post.provider_post_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();

    let qualification = UpstreamScanQualification {
        provider: "twitter",
        source_id: &context.source.id,
        sections_scanned: &sections,
        observed_post_keys: &observed_post_keys,
        enumerated_in_full: request.incremental_cutoff_timestamp.is_none()
            && request.resume_cursors.is_empty(),
        // Unlike Instagram, the X sync exposes no date window to narrow a
        // listing; the incremental cutoff above is the only narrowing knob.
        filtered: false,
        // `incomplete_post_count` counts posts whose assets did not all arrive.
        // They are dropped from `observed_posts`, so treating the scan as whole
        // would flag posts that were merely incomplete this run.
        truncated: result.manifest_summary.rate_limited
            || result.manifest_summary.incomplete_post_count > 0,
    };

    if let Err(error) = evaluate_upstream_presence(connection, &qualification, timestamp) {
        let _ = runtime_log::append_workspace(
            "upstream_presence",
            "warn",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some("twitter"),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            "Failed to evaluate which posts are gone from the provider.",
            Some(error),
        );
    }
}

/// Wires the TikTok sync into the presence evaluation.
///
/// TikTok has no incremental discovery stop — yt-dlp lists the profile every
/// run — so any scan that was not narrowed by a date window and did not hit a
/// limit qualifies. Only the timeline and reposts are judged: stories expire on
/// their own and likes disappear when the owner un-likes them.
pub(super) fn evaluate_tiktok_upstream_presence(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &tiktok_connector::TikTokConnectorRequest,
    result: &tiktok_connector::TikTokConnectorResult,
    timestamp: &str,
) {
    let mut sections = Vec::new();
    if request.sections.timeline {
        sections.push("timeline".to_string());
    }
    if request.sections.reposts {
        sections.push("reposts".to_string());
    }

    let observed_post_keys = result
        .observed_posts
        .iter()
        .map(|post| post.provider_post_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();

    let qualification = UpstreamScanQualification {
        provider: "tiktok",
        source_id: &context.source.id,
        sections_scanned: &sections,
        observed_post_keys: &observed_post_keys,
        enumerated_in_full: request.target_video_url.is_none(),
        filtered: request.download_from_date.is_some_and(|value| value > 0)
            || request.download_to_date.is_some_and(|value| value > 0),
        truncated: result.rate_limited
            || result.limit_aborted
            || !result.section_errors.is_empty()
            // A rename or a duplicate user id means the listing that came back
            // belongs to a profile identity still being resolved.
            || result.resolved_handle.is_some()
            || result.duplicate_user_id.is_some(),
    };

    if let Err(error) = evaluate_upstream_presence(connection, &qualification, timestamp) {
        let _ = runtime_log::append_workspace(
            "upstream_presence",
            "warn",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some("tiktok"),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            "Failed to evaluate which posts are gone from the provider.",
            Some(error),
        );
    }
}

/// Wires the YouTube sync into the presence evaluation. Same shape as TikTok:
/// yt-dlp lists the channel every run, so a scan that neither hit a limit nor
/// errored covers the whole section.
pub(super) fn evaluate_youtube_upstream_presence(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &youtube_connector::YouTubeConnectorRequest,
    result: &youtube_connector::YouTubeConnectorResult,
    timestamp: &str,
) {
    let mut sections = Vec::new();
    if request.sections.videos {
        sections.push("videos".to_string());
    }
    if request.sections.shorts {
        sections.push("shorts".to_string());
    }

    let observed_post_keys = result
        .observed_posts
        .iter()
        .map(|post| post.provider_post_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();

    let qualification = UpstreamScanQualification {
        provider: "youtube",
        source_id: &context.source.id,
        sections_scanned: &sections,
        observed_post_keys: &observed_post_keys,
        enumerated_in_full: true,
        filtered: false,
        truncated: result.rate_limited
            || result.limit_aborted
            || result.profile_unavailable
            || !result.section_errors.is_empty()
            || result.duplicate_user_id.is_some(),
    };

    if let Err(error) = evaluate_upstream_presence(connection, &qualification, timestamp) {
        let _ = runtime_log::append_workspace(
            "upstream_presence",
            "warn",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some("youtube"),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            "Failed to evaluate which posts are gone from the provider.",
            Some(error),
        );
    }
}

/// Wires the VSCO sync into the presence evaluation. Only the gallery is
/// judged; journal entries are long-form collections whose listing behaviour is
/// not verified here.
pub(super) fn evaluate_vsco_upstream_presence(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &vsco_connector::VscoConnectorRequest,
    result: &vsco_connector::VscoConnectorResult,
    timestamp: &str,
) {
    let mut sections = Vec::new();
    if request.sections.gallery {
        sections.push("gallery".to_string());
    }

    let observed_post_keys = result
        .observed_posts
        .iter()
        .map(|post| post.provider_post_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();

    let qualification = UpstreamScanQualification {
        provider: "vsco",
        source_id: &context.source.id,
        sections_scanned: &sections,
        observed_post_keys: &observed_post_keys,
        enumerated_in_full: true,
        filtered: false,
        truncated: !result.section_errors.is_empty() || result.duplicate_user_id.is_some(),
    };

    if let Err(error) = evaluate_upstream_presence(connection, &qualification, timestamp) {
        let _ = runtime_log::append_workspace(
            "upstream_presence",
            "warn",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some("vsco"),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            "Failed to evaluate which posts are gone from the provider.",
            Some(error),
        );
    }
}

/// Post keys the gallery should badge as archived-only.
pub(crate) fn load_upstream_missing_post_keys(
    connection: &Connection,
    provider: &str,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    let table = post_ledger_table(provider);
    let provider_clause = provider_scope_clause(provider).replace("?provider", "?1");
    let statement = format!(
        "SELECT provider_post_key FROM {table}
         WHERE source_id = ?2{provider_clause} AND upstream_state = 'missing'"
    );
    let mut prepared = connection
        .prepare(&statement)
        .map_err(|error| error.to_string())?;
    let rows = prepared
        .query_map(params![provider, source_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row.map_err(|error| error.to_string())?);
    }
    Ok(keys)
}
