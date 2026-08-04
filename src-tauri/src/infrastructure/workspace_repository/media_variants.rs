use super::*;

/// Hamming distance below which two perceptual hashes are treated as the same
/// image. 64-bit dHash/aHash: identical content re-encoded typically lands under
/// 6, unrelated images sit far above 20.
const PERCEPTUAL_DISTANCE_THRESHOLD: u32 = 6;

/// Videos are matched on sampled-frame hashes; a majority of matching frames is
/// what makes a story and its feed repost the same video despite different
/// crops, bitrates and lengths.
const VIDEO_FRAME_MATCH_RATIO: f64 = 0.6;

/// How far apart two posts from different providers may be and still be
/// considered the same upload. Cross-posting happens within hours, not months,
/// and the window keeps the candidate set small.
const CROSS_SOURCE_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone)]
struct VariantCandidate {
    media_id: String,
    source_id: String,
    media_section: String,
    media_type: String,
    sha256: Option<String>,
    ahash64: Option<String>,
    dhash64: Option<String>,
    video_signature: Option<String>,
    captured_at: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    duration_ms: Option<i64>,
    size_bytes: i64,
}

fn hamming_distance(left: &str, right: &str) -> Option<u32> {
    let left = u64::from_str_radix(left.trim(), 16).ok()?;
    let right = u64::from_str_radix(right.trim(), 16).ok()?;
    Some((left ^ right).count_ones())
}

fn frame_hashes(signature: Option<&str>) -> Vec<String> {
    signature
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

/// Fraction of sampled frames that match between two video signatures.
fn video_similarity(left: Option<&str>, right: Option<&str>) -> f64 {
    let left = frame_hashes(left);
    let right = frame_hashes(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut matched = 0usize;
    for hash in &left {
        if right
            .iter()
            .any(|other| hamming_distance(hash, other).is_some_and(|d| d <= PERCEPTUAL_DISTANCE_THRESHOLD))
        {
            matched += 1;
        }
    }
    matched as f64 / left.len().max(right.len()) as f64
}

fn images_match(left: &VariantCandidate, right: &VariantCandidate) -> bool {
    // Aspect ratio guards against matching a portrait crop with its landscape
    // sibling purely on a coarse hash.
    let ratio = |candidate: &VariantCandidate| match (candidate.width, candidate.height) {
        (Some(width), Some(height)) if height > 0 => Some(width as f64 / height as f64),
        _ => None,
    };
    if let (Some(a), Some(b)) = (ratio(left), ratio(right)) {
        if (a - b).abs() > 0.15 {
            return false;
        }
    }
    let compare = |left: &Option<String>, right: &Option<String>| match (left, right) {
        (Some(left), Some(right)) => hamming_distance(left, right),
        _ => None,
    };
    compare(&left.dhash64, &right.dhash64)
        .or_else(|| compare(&left.ahash64, &right.ahash64))
        .is_some_and(|distance| distance <= PERCEPTUAL_DISTANCE_THRESHOLD)
}

fn videos_match(left: &VariantCandidate, right: &VariantCandidate) -> bool {
    if let (Some(a), Some(b)) = (left.duration_ms, right.duration_ms) {
        // A story is often a trimmed cut of the feed post; allow a generous
        // spread but reject obviously different videos.
        let longest = a.max(b) as f64;
        if longest > 0.0 && ((a - b).abs() as f64 / longest) > 0.35 {
            return false;
        }
    }
    video_similarity(left.video_signature.as_deref(), right.video_signature.as_deref())
        >= VIDEO_FRAME_MATCH_RATIO
}

fn match_kind(left: &VariantCandidate, right: &VariantCandidate) -> Option<(&'static str, f64)> {
    if let (Some(left_hash), Some(right_hash)) = (left.sha256.as_deref(), right.sha256.as_deref()) {
        if left_hash.eq_ignore_ascii_case(right_hash) {
            return Some(("exact_sha256", 1.0));
        }
    }
    if left.media_type != right.media_type {
        return None;
    }
    if left.media_type == "video" {
        return videos_match(left, right).then_some(("perceptual_video", 0.85));
    }
    images_match(left, right).then_some(("perceptual_image", 0.9))
}

fn load_candidates(
    connection: &Connection,
    source_ids: &[String],
) -> Result<Vec<VariantCandidate>, String> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = source_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let statement = format!(
        "SELECT id, source_id, media_section, media_type, sha256, ahash64, dhash64,
                video_signature, captured_at, width, height, duration_ms, size_bytes
         FROM media_index
         WHERE source_id IN ({placeholders})
           AND local_state = 'present'
           AND variant_group_id IS NULL
           AND fingerprint_status = 'complete'
           AND (sha256 IS NOT NULL OR ahash64 IS NOT NULL OR dhash64 IS NOT NULL
                OR video_signature IS NOT NULL)
         ORDER BY captured_at, id"
    );
    let mut prepared = connection
        .prepare(&statement)
        .map_err(|error| error.to_string())?;
    let bindings: Vec<&dyn rusqlite::ToSql> = source_ids
        .iter()
        .map(|value| value as &dyn rusqlite::ToSql)
        .collect();
    let rows = prepared
        .query_map(bindings.as_slice(), |row| {
            Ok(VariantCandidate {
                media_id: row.get(0)?,
                source_id: row.get(1)?,
                media_section: row.get(2)?,
                media_type: row.get(3)?,
                sha256: row.get(4)?,
                ahash64: row.get(5)?,
                dhash64: row.get(6)?,
                video_signature: row.get(7)?,
                captured_at: row.get(8)?,
                width: row.get(9)?,
                height: row.get(10)?,
                duration_ms: row.get(11)?,
                size_bytes: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Seven disjoint partitions guarantee that hashes at Hamming distance <= 6
/// share at least one exact partition (pigeonhole principle). This keeps the
/// candidate generator complete without comparing every image with every other
/// image; the existing matcher still confirms distance and aspect ratio.
fn perceptual_bucket_keys(hash: &str) -> Vec<(u8, u16)> {
    let Ok(value) = u64::from_str_radix(hash.trim(), 16) else {
        return Vec::new();
    };
    let mut keys = Vec::with_capacity(7);
    let mut shift = 0_u32;
    for partition in 0..7_u8 {
        let width = if partition == 6 { 10 } else { 9 };
        let mask = (1_u64 << width) - 1;
        keys.push((partition, ((value >> shift) & mask) as u16));
        shift += width;
    }
    keys
}

fn add_perceptual_candidate(
    index: usize,
    hash: &str,
    buckets: &mut HashMap<(u8, u16), Vec<usize>>,
    pairs: &mut HashSet<(usize, usize)>,
) {
    for key in perceptual_bucket_keys(hash) {
        let bucket = buckets.entry(key).or_default();
        for &other in bucket.iter() {
            if other != index {
                pairs.insert((other.min(index), other.max(index)));
            }
        }
        if bucket.last().copied() != Some(index) {
            bucket.push(index);
        }
    }
}

fn comparison_pairs(candidates: &[VariantCandidate]) -> Vec<Vec<usize>> {
    let mut pairs: HashSet<(usize, usize)> = HashSet::new();
    let mut exact: HashMap<String, Vec<usize>> = HashMap::new();
    let mut images: HashMap<(u8, u16), Vec<usize>> = HashMap::new();
    let mut videos: HashMap<(u8, u16), Vec<usize>> = HashMap::new();

    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(sha256) = candidate.sha256.as_deref() {
            let bucket = exact.entry(sha256.to_ascii_lowercase()).or_default();
            for &other in bucket.iter() {
                pairs.insert((other, index));
            }
            bucket.push(index);
        }
        if candidate.media_type == "video" {
            let mut seen_keys = HashSet::new();
            for hash in frame_hashes(candidate.video_signature.as_deref()) {
                for key in perceptual_bucket_keys(&hash) {
                    if seen_keys.insert(key) {
                        let bucket = videos.entry(key).or_default();
                        for &other in bucket.iter() {
                            if other != index {
                                pairs.insert((other.min(index), other.max(index)));
                            }
                        }
                        bucket.push(index);
                    }
                }
            }
        } else if let Some(hash) = candidate.dhash64.as_deref().or(candidate.ahash64.as_deref()) {
            add_perceptual_candidate(index, hash, &mut images, &mut pairs);
        }
    }

    let mut adjacent = vec![Vec::new(); candidates.len()];
    for (left, right) in pairs {
        adjacent[left].push(right);
    }
    for neighbors in &mut adjacent {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    adjacent
}

/// The best copy to keep as the group's canonical member: highest resolution,
/// then largest file. The story crop loses to the clean feed upload, which is
/// what an archive wants to show by default.
fn pick_canonical(members: &[VariantCandidate]) -> String {
    members
        .iter()
        .max_by_key(|candidate| {
            (
                candidate.width.unwrap_or(0) * candidate.height.unwrap_or(0),
                candidate.size_bytes,
            )
        })
        .map(|candidate| candidate.media_id.clone())
        .unwrap_or_default()
}

fn persist_group(
    connection: &Connection,
    scope: &str,
    identity_id: Option<&str>,
    kind: &str,
    confidence: f64,
    members: &[VariantCandidate],
    timestamp: &str,
) -> Result<(), String> {
    let canonical = pick_canonical(members);
    let group_id = Uuid::new_v4().to_string();
    connection
        .execute(
            "INSERT INTO media_variant_groups (
                id, scope, identity_id, canonical_media_id, match_kind, confidence,
                policy_applied, reviewed, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'link_only', 0, ?7, ?7)",
            params![
                group_id,
                scope,
                identity_id,
                canonical,
                kind,
                confidence,
                timestamp
            ],
        )
        .map_err(|error| error.to_string())?;

    for member in members {
        let role = if member.media_id == canonical {
            "canonical"
        } else {
            "variant"
        };
        connection
            .execute(
                "INSERT INTO media_variant_members (group_id, media_id, similarity, role)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(group_id, media_id) DO NOTHING",
                params![group_id, member.media_id, confidence, role],
            )
            .map_err(|error| error.to_string())?;
        // `link_only`: nothing is moved or deleted. Only the canonical flag on
        // the index changes, so the gallery can collapse the group into one card.
        connection
            .execute(
                "UPDATE media_index
                 SET variant_group_id = ?2, is_canonical = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![
                    member.media_id,
                    group_id,
                    i64::from(role == "canonical"),
                    timestamp
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct VariantDetectionOutcome {
    pub(crate) groups_created: usize,
    pub(crate) media_grouped: usize,
}

/// Finds duplicate content within one profile (a story reposted to the feed) and
/// across the profiles of one identity (the same upload on two providers).
///
/// Only media with a completed fingerprint takes part — an un-hashed file is
/// simply not comparable yet, and guessing would be worse than waiting.
pub(super) fn detect_variants_with_connection(
    connection: &Connection,
    source_ids: &[String],
    identity_id: Option<&str>,
    timestamp: &str,
) -> Result<VariantDetectionOutcome, String> {
    let candidates = load_candidates(connection, source_ids)?;
    let adjacent = comparison_pairs(&candidates);
    let mut outcome = VariantDetectionOutcome::default();
    let mut claimed: HashSet<String> = HashSet::new();

    for (index, candidate) in candidates.iter().enumerate() {
        if claimed.contains(&candidate.media_id) {
            continue;
        }
        let mut members = vec![candidate.clone()];
        let mut kind = "";
        let mut confidence = 0.0;

        for &other_index in &adjacent[index] {
            let other = &candidates[other_index];
            if claimed.contains(&other.media_id) {
                continue;
            }
            let cross_source = other.source_id != candidate.source_id;
            // Two posts of the same profile in the same section are simply two
            // posts; only a section change (story → feed) or a different
            // provider makes them a repost worth grouping.
            if !cross_source && other.media_section == candidate.media_section {
                continue;
            }
            if cross_source {
                match (candidate.captured_at, other.captured_at) {
                    (Some(left), Some(right))
                        if (left - right).abs() > CROSS_SOURCE_WINDOW_SECONDS =>
                    {
                        continue
                    }
                    _ => {}
                }
            }
            if let Some((matched_kind, matched_confidence)) = match_kind(candidate, other) {
                kind = matched_kind;
                confidence = matched_confidence;
                members.push(other.clone());
            }
        }

        if members.len() < 2 {
            continue;
        }
        let scope = if members
            .iter()
            .any(|member| member.source_id != candidate.source_id)
        {
            "cross_source"
        } else {
            "intra_source"
        };
        for member in &members {
            claimed.insert(member.media_id.clone());
        }
        persist_group(
            connection,
            scope,
            identity_id,
            kind,
            confidence,
            &members,
            timestamp,
        )?;
        outcome.groups_created += 1;
        outcome.media_grouped += members.len();
    }

    Ok(outcome)
}

/// Processes every standalone profile and linked identity exactly once. The
/// previous caller invoked identity-wide detection once per member profile,
/// which repeated the same comparisons and could publish duplicate groups.
pub(crate) fn detect_variants_for_all_scopes() -> Result<VariantDetectionOutcome, String> {
    with_workspace(|connection, _| {
        let sources = load_sources(connection)?;
        let mut identity_sources: HashMap<String, Vec<String>> = HashMap::new();
        let mut standalone = Vec::new();
        for source in sources {
            if let Some(identity_id) = source.identity_id {
                identity_sources.entry(identity_id).or_default().push(source.id);
            } else {
                standalone.push(source.id);
            }
        }
        let timestamp = Utc::now().to_rfc3339();
        let mut total = VariantDetectionOutcome::default();
        for source_id in standalone {
            let outcome = detect_variants_with_connection(
                connection,
                &[source_id],
                None,
                &timestamp,
            )?;
            total.groups_created += outcome.groups_created;
            total.media_grouped += outcome.media_grouped;
        }
        for (identity_id, source_ids) in identity_sources {
            let outcome = detect_variants_with_connection(
                connection,
                &source_ids,
                Some(&identity_id),
                &timestamp,
            )?;
            total.groups_created += outcome.groups_created;
            total.media_grouped += outcome.media_grouped;
        }
        Ok(total)
    })
}

pub(crate) fn load_variant_groups(limit: Option<u32>) -> Result<Vec<MediaVariantGroup>, String> {
    with_workspace(|connection, layout| {
        load_variant_groups_with_connection(connection, layout, limit.unwrap_or(100))
    })
}

pub(super) fn load_variant_groups_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    limit: u32,
) -> Result<Vec<MediaVariantGroup>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, scope, identity_id, canonical_media_id, match_kind, confidence,
                    policy_applied, reviewed, created_at
             FROM media_variant_groups
             ORDER BY reviewed, created_at DESC
             LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok(MediaVariantGroup {
                id: row.get(0)?,
                scope: row.get(1)?,
                identity_id: row.get(2)?,
                canonical_media_id: row.get(3)?,
                match_kind: row.get(4)?,
                confidence: row.get(5)?,
                policy_applied: row.get(6)?,
                reviewed: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
                members: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?;
    let mut groups = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let sources = load_sources(connection)?
        .into_iter()
        .map(|source| (source.id.clone(), source))
        .collect::<HashMap<_, _>>();
    let mut roots: HashMap<String, PathBuf> = HashMap::new();

    for group in &mut groups {
        let mut members = connection
            .prepare(
                "SELECT media_variant_members.media_id, media_variant_members.role,
                        media_index.source_id, media_index.provider, media_index.media_section,
                        media_index.relative_path, media_index.media_type, media_index.size_bytes,
                        source_profiles.handle
                 FROM media_variant_members
                 JOIN media_index ON media_index.id = media_variant_members.media_id
                 JOIN source_profiles ON source_profiles.id = media_index.source_id
                 WHERE media_variant_members.group_id = ?1
                 ORDER BY media_variant_members.role",
            )
            .map_err(|error| error.to_string())?;
        let rows = members
            .query_map(params![group.id], |row| {
                Ok(MediaVariantMember {
                    media_id: row.get(0)?,
                    role: row.get(1)?,
                    source_id: row.get(2)?,
                    provider: row.get(3)?,
                    media_section: row.get(4)?,
                    relative_path: row.get(5)?,
                    absolute_path: String::new(),
                    media_type: row.get(6)?,
                    size_bytes: row.get(7)?,
                    handle: row.get(8)?,
                })
            })
            .map_err(|error| error.to_string())?;
        group.members = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for member in &mut group.members {
            if !roots.contains_key(&member.source_id) {
                if let Some(source) = sources.get(&member.source_id) {
                    let root = resolved_source_media_output_root_with_connection(
                        connection,
                        layout,
                        source,
                    )?;
                    roots.insert(member.source_id.clone(), root);
                }
            }
            if let Some(root) = roots.get(&member.source_id) {
                member.absolute_path = root
                    .join(&member.relative_path)
                    .to_string_lossy()
                    .to_string();
            }
        }
    }
    Ok(groups)
}

/// Undoes a grouping the operator disagrees with: the members go back to being
/// independent canonical media.
pub(crate) fn dismiss_variant_group(group_id: String) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_index
                 SET variant_group_id = NULL, is_canonical = 1, updated_at = ?2
                 WHERE variant_group_id = ?1",
                params![group_id, Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM media_variant_groups WHERE id = ?1",
                params![group_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}
