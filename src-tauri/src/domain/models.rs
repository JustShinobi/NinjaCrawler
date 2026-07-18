use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    pub key: String,
    pub display_name: String,
    pub auth_modes: Vec<String>,
    pub supports_multi_account: bool,
    pub source_kinds: Vec<String>,
    pub default_capabilities: Vec<String>,
    pub notes: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub auth_mode: String,
    pub auth_state: String,
    pub capabilities: Vec<String>,
    pub last_validated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountSession {
    pub account_id: String,
    pub auth_mode: String,
    pub session_format: String,
    pub fingerprint: String,
    pub cookie_count: u32,
    pub imported_at: String,
    pub last_validated_at: Option<String>,
    pub last_validation_error: Option<String>,
    pub has_secret: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expires_at: Option<String>,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionAccountIdentity {
    pub provider_user_id: Option<String>,
    pub username: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionAccountCapture {
    pub provider: String,
    pub current_url: String,
    pub identity: CompanionAccountIdentity,
    pub cookies: Vec<ProviderAccountCookie>,
    #[serde(default)]
    pub authorization: HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompanionAccountCandidate {
    pub account_id: String,
    pub display_name: String,
    pub match_kind: Option<String>,
    pub has_session: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompanionAccountPreview {
    pub provider: String,
    pub username: String,
    pub cookie_count: usize,
    pub authorization_fields: Vec<String>,
    pub missing_required_fields: Vec<String>,
    pub candidates: Vec<CompanionAccountCandidate>,
    pub suggested_account_id: Option<String>,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionAccountImportInput {
    pub capture: CompanionAccountCapture,
    pub target_account_id: Option<String>,
    pub create_display_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompanionAccountImportResult {
    pub account_id: String,
    pub created: bool,
    pub auth_state: String,
    pub validation_error: Option<String>,
    pub can_revert: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountImportState {
    pub account_id: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub last_imported_at: String,
    pub can_revert: bool,
    pub backup_imported_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountSettingValueKind {
    String,
    Json,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountSettingValue {
    pub setting_key: String,
    pub value_kind: ProviderAccountSettingValueKind,
    pub string_value: Option<String>,
    pub json_value: Option<serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountEditor {
    pub account: ProviderAccount,
    pub session: Option<ProviderAccountSession>,
    pub settings: Vec<ProviderAccountSettingValue>,
    pub import_state: Option<ProviderAccountImportState>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProviderDescriptor {
    pub key: String,
    pub display_name: String,
    pub description: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportMethodDescriptor {
    pub importer_id: String,
    pub provider: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRootDescriptor {
    pub path: String,
    pub source: String,
    pub label: String,
    pub removable: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProblem {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewProfile {
    pub profile_root: String,
    pub user_xml_path: String,
    pub handle: String,
    pub display_name: String,
    pub account_name: Option<String>,
    pub source_id: Option<String>,
    pub source_display_name: Option<String>,
    pub source_handle: Option<String>,
    pub account_id: Option<String>,
    pub account_display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub already_imported: bool,
    pub import_state: String,
    pub file_count: u32,
    pub already_cataloged_count: u32,
    pub new_file_count: u32,
    pub problems: Vec<ImportProblem>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewSummary {
    pub detected_profiles: u32,
    pub ready_profiles: u32,
    pub blocked_profiles: u32,
    pub already_imported_profiles: u32,
    pub importable_files: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub importer_id: String,
    pub provider: String,
    pub method_label: String,
    pub force_reimport: bool,
    pub roots: Vec<String>,
    pub profiles: Vec<ImportPreviewProfile>,
    pub summary: ImportPreviewSummary,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewOptions {
    pub force_reimport: bool,
    #[serde(default)]
    pub manual_roots: Vec<String>,
    #[serde(default)]
    pub disabled_roots: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResolution {
    pub profile_root: String,
    pub action: String,
    pub account_id: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunRequest {
    pub force_reimport: bool,
    #[serde(default)]
    pub manual_roots: Vec<String>,
    #[serde(default)]
    pub disabled_roots: Vec<String>,
    pub resolutions: Vec<ImportResolution>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunProfileResult {
    pub profile_root: String,
    pub handle: String,
    pub status: String,
    pub source_id: Option<String>,
    pub imported_media_count: u32,
    pub already_cataloged_count: u32,
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunResult {
    pub importer_id: String,
    pub imported_profiles: u32,
    pub skipped_profiles: u32,
    pub failed_profiles: u32,
    pub imported_media_count: u32,
    pub already_cataloged_count: u32,
    pub profiles: Vec<ImportRunProfileResult>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstagramNamingLedgerBackfillResult {
    pub scanned_sources: u32,
    pub scanned_profiles: u32,
    pub scanned_files: u32,
    pub inserted_entries: u32,
    pub updated_entries: u32,
    pub skipped_files: u32,
    pub legacy_records_total: u32,
    pub legacy_records_matched: u32,
    pub legacy_records_missing_files: u32,
    pub backfilled_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportQueueJob {
    pub job_id: String,
    pub importer_id: String,
    pub provider: String,
    pub method_label: String,
    pub job_kind: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress_percent: Option<u32>,
    pub progress_label: Option<String>,
    pub progress_detail: Option<String>,
    pub progress_indeterminate: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportQueueRecentResult {
    pub job_id: String,
    pub importer_id: String,
    pub provider: String,
    pub method_label: String,
    pub job_kind: String,
    pub status: String,
    pub summary: String,
    pub finished_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub total_count: u32,
    pub active_job_id: Option<String>,
    pub active_importer_id: Option<String>,
    pub active_provider: Option<String>,
    pub active_method_label: Option<String>,
    pub active_job_kind: Option<String>,
    pub active_started_at: Option<String>,
    pub queued_items: Vec<ImportQueueJob>,
    pub running_items: Vec<ImportQueueJob>,
    pub recent_results: Vec<ImportQueueRecentResult>,
    pub latest_preview: Option<ImportPreview>,
    pub latest_run_result: Option<ImportRunResult>,
    pub latest_backfill_result: Option<InstagramNamingLedgerBackfillResult>,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogEntry {
    pub id: String,
    pub timestamp: String,
    pub scope: String,
    pub level: String,
    pub account_id: Option<String>,
    pub provider: Option<String>,
    pub source_id: Option<String>,
    pub source_handle: Option<String>,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDebugEntry {
    pub id: String,
    pub timestamp: String,
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub source_handle: Option<String>,
    pub connector: String,
    pub event_type: String,
    pub operation: String,
    pub raw: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogContext {
    pub provider_catalog: Vec<ProviderDescriptor>,
    pub accounts: Vec<ProviderAccount>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogWindowStatus {
    pub window_open: bool,
    pub open_requests: u64,
    pub ready_signals: u64,
    pub last_ready_at: Option<String>,
    pub last_failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountsWindowIntent {
    pub initial_account_id: Option<String>,
    pub initial_provider: Option<String>,
    pub initial_mode: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEditorSeedIntent {
    pub provider: String,
    pub handle: String,
    pub display_name: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceEditorWindowIntent {
    pub source_id: Option<String>,
    pub preferred_provider: Option<String>,
    pub preferred_account_id: Option<String>,
    pub seed: Option<SourceEditorSeedIntent>,
}

pub type ProfileEditorSeedIntent = SourceEditorSeedIntent;
pub type ProfileEditorWindowIntent = SourceEditorWindowIntent;

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanEditorWindowIntent {
    pub mode: Option<String>,
    pub plan_id: Option<String>,
    pub scheduler_set_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaGalleryFile {
    pub relative_path: String,
    pub absolute_path: String,
    pub media_type: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaGalleryPost {
    pub post_id: Option<String>,
    pub post_url: Option<String>,
    pub captured_at: Option<i64>,
    /// Momento (unix) em que a mídia foi baixada/vista pela 1ª vez pelo app —
    /// derivado do `first_seen_at` do ledger, com fallback para o mtime do
    /// arquivo. Usado como eixo alternativo de ordenação (Download Date).
    pub downloaded_at: Option<i64>,
    /// Autor original do conteúdo. Só é preenchido nos Likes do TikTok (o dono
    /// do perfil curtiu o vídeo de outra pessoa); usado para busca por autor.
    pub author: Option<String>,
    /// "video" | "image" | "slideshow"
    pub media_type: String,
    /// Subpasta do perfil ("timeline"/raiz, "stories", "reposts", "video").
    pub section: String,
    /// Álbuns de highlight a que o post pertence (subpasta sob `Stories/` e/ou
    /// associações de `instagram_highlight_membership`). Um post do Feed pode
    /// pertencer a vários destaques sem ter o arquivo duplicado em disco.
    pub albums: Vec<String>,
    /// Caminho absoluto de uma miniatura/poster (cover do vídeo, quando houver).
    pub poster_path: Option<String>,
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub share_count: Option<i64>,
    pub stats_updated_at: Option<String>,
    pub files: Vec<MediaGalleryFile>,
}

/// Resultado da pré-checagem de migrations no boot: presente só quando há
/// migrations pendentes num banco que já tem schema (dispara a tela de migração).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatus {
    pub from_version: i64,
    pub to_version: i64,
    pub pending_count: usize,
    pub db_size_bytes: u64,
}

/// Progresso emitido durante a migração (backup + aplicação das migrations).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProgress {
    /// `backup` (snapshot do banco) ou `migrate` (aplicando as migrations).
    pub phase: String,
    pub current: u64,
    pub total: u64,
    pub label: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMediaGallery {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub profile_url: String,
    pub posts: Vec<MediaGalleryPost>,
    /// Metadados de perfil da última sincronização (Instagram hoje), para o
    /// cabeçalho enriquecido. Todos opcionais — perfis importados/sem sync ainda
    /// não os têm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biography: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follower_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_updated_at: Option<String>,
}

/// Lote de thumbnails de vídeo gerados sob demanda (ffmpeg) para o grid do
/// Profile View. `available=false` sinaliza que o ffmpeg não está instalado —
/// o front cai no thumb por `<video>` de antes.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailBatch {
    pub available: bool,
    /// caminho absoluto do vídeo → caminho absoluto do jpg em cache.
    pub thumbs: HashMap<String, String>,
}

/// Thumbnail de avatar (jpg 256px) em cache local — a lista de perfis exibe
/// estes arquivos pequenos no disco do sistema em vez dos `ProfilePicture.jpg`
/// em resolução original no volume de mídia.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarThumbnail {
    pub source_id: String,
    pub path: String,
    /// mtime (ms) do jpg — cache-buster `?v=` no frontend, já que o path do
    /// thumb é estável por convenção `{source_id}.jpg`.
    pub version: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarThumbnailBatch {
    pub thumbs: Vec<AvatarThumbnail>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailQueueItem {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub files_scanned: u32,
    pub files_total: u32,
    pub files_processed: u32,
    pub generated: u32,
    pub skipped_existing: u32,
    pub failed: u32,
    pub current_file: Option<String>,
    pub progress_percent: Option<u32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailQueueResult {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub status: String,
    pub summary: String,
    pub generated: u32,
    pub skipped_existing: u32,
    pub failed: u32,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaThumbnailQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub active: Option<MediaThumbnailQueueItem>,
    pub queued_items: Vec<MediaThumbnailQueueItem>,
    pub recent_results: Vec<MediaThumbnailQueueResult>,
    pub updated_at: String,
}

/// Vídeo avulso capturado por URL (via Companion), fora da estrutura de perfis.
/// Fica numa raiz "Single videos" plana; o catálogo guarda os metadados para o
/// media view filtrar por provider/autor/data.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleVideo {
    pub id: String,
    pub provider: String,
    pub source_url: String,
    pub provider_video_id: Option<String>,
    pub uploader: Option<String>,
    pub title: Option<String>,
    pub relative_path: String,
    pub absolute_path: String,
    pub media_type: String,
    pub captured_at: Option<i64>,
    pub downloaded_at: String,
    pub files: Vec<SingleVideoFile>,
    pub audio_relative_path: Option<String>,
    pub audio_absolute_path: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleVideoFile {
    pub relative_path: String,
    pub absolute_path: String,
    pub media_type: String,
}

/// Item da fila leve de downloads de single video (um worker sequencial). Sem
/// persistência: um download interrompido pelo fechamento do app é perdido (o
/// usuário reenvia a URL).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleVideoQueueItem {
    pub id: String,
    pub url: String,
    pub provider: Option<String>,
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress_label: Option<String>,
    pub progress_indeterminate: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleVideoQueueRecentResult {
    pub url: String,
    pub provider: Option<String>,
    pub uploader: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub summary: String,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleVideoQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub active: Option<SingleVideoQueueItem>,
    pub queued_items: Vec<SingleVideoQueueItem>,
    pub recent_results: Vec<SingleVideoQueueRecentResult>,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncQueueProviderStatus {
    pub provider: String,
    pub display_name: String,
    pub queued: u32,
    pub running: u32,
    pub completed: u32,
    pub failed: u32,
    pub total: u32,
    /// Progresso do download em andamento deste provider (0-100), quando há um
    /// job rodando com percentual conhecido.
    pub active_progress_percent: Option<u32>,
    /// O provider está pausado: jobs em fila não iniciam até retomar.
    pub paused: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncQueueItem {
    pub job_key: String,
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub account_id: Option<String>,
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress_percent: Option<u32>,
    pub progress_label: Option<String>,
    pub progress_detail: Option<String>,
    pub progress_indeterminate: bool,
    pub downloaded_items: Option<u32>,
    /// Prazo do hold automatico da Account, quando o item esta aguardando rate
    /// limit. Ausente para jobs normais e pausas manuais por provider.
    pub hold_until: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncQueueRecentResult {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub account_id: Option<String>,
    pub status: String,
    pub summary: String,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub total_count: u32,
    pub active_source_id: Option<String>,
    pub active_handle: Option<String>,
    pub active_provider: Option<String>,
    pub active_started_at: Option<String>,
    pub providers: Vec<SourceSyncQueueProviderStatus>,
    pub queued_items: Vec<SourceSyncQueueItem>,
    pub running_items: Vec<SourceSyncQueueItem>,
    pub recent_results: Vec<SourceSyncQueueRecentResult>,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDeleteQueueJob {
    pub job_id: String,
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub mode: SourceProfileDeleteMode,
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress_percent: Option<u32>,
    pub progress_label: Option<String>,
    pub progress_detail: Option<String>,
    pub progress_indeterminate: bool,
    pub files_processed: Option<u32>,
    pub files_total: Option<u32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDeleteQueueRecentResult {
    pub job_id: String,
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub mode: SourceProfileDeleteMode,
    pub status: String,
    pub summary: String,
    pub finished_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDeleteQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub total_count: u32,
    pub active_job_id: Option<String>,
    pub active_source_id: Option<String>,
    pub active_handle: Option<String>,
    pub active_provider: Option<String>,
    pub active_mode: Option<SourceProfileDeleteMode>,
    pub active_started_at: Option<String>,
    pub queued_items: Vec<SourceDeleteQueueJob>,
    pub running_items: Vec<SourceDeleteQueueJob>,
    pub recent_results: Vec<SourceDeleteQueueRecentResult>,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPathMigrationQueueJob {
    pub job_id: String,
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub source_path: String,
    pub target_path: String,
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress_percent: Option<u32>,
    pub progress_stage: String,
    pub progress_indeterminate: bool,
    pub progress_label: Option<String>,
    pub progress_detail: Option<String>,
    pub files_processed: u64,
    pub files_total: u64,
    pub bytes_processed: u64,
    pub bytes_total: u64,
    pub current_file: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPathMigrationQueueRecentResult {
    pub job_id: String,
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub source_path: String,
    pub target_path: String,
    pub status: String,
    pub summary: String,
    pub finished_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPathMigrationQueueStatus {
    pub queued_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    pub total_count: u32,
    pub queued_items: Vec<MediaPathMigrationQueueJob>,
    pub running_items: Vec<MediaPathMigrationQueueJob>,
    pub recent_results: Vec<MediaPathMigrationQueueRecentResult>,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramSourceSyncPresetSections {
    pub timeline: bool,
    pub reels: bool,
    pub stories: bool,
    pub stories_user: bool,
    pub tagged: bool,
}

impl Default for InstagramSourceSyncPresetSections {
    fn default() -> Self {
        Self {
            timeline: true,
            reels: false,
            stories: false,
            stories_user: false,
            tagged: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramExtractImageFromVideoSections {
    pub timeline: bool,
    pub reels: bool,
    pub stories: bool,
    pub stories_user: bool,
    pub tagged: bool,
}

impl Default for InstagramExtractImageFromVideoSections {
    fn default() -> Self {
        Self {
            timeline: true,
            reels: true,
            stories: true,
            stories_user: true,
            tagged: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramSourceSyncPreset {
    pub enabled: bool,
    pub label: String,
    pub sections: InstagramSourceSyncPresetSections,
}

impl InstagramSourceSyncPreset {
    pub fn preset1() -> Self {
        Self {
            enabled: false,
            label: "Preset 1".to_string(),
            sections: InstagramSourceSyncPresetSections::default(),
        }
    }

    pub fn preset2() -> Self {
        Self {
            enabled: false,
            label: "Preset 2".to_string(),
            sections: InstagramSourceSyncPresetSections::default(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramSourceSyncOptions {
    pub timeline: bool,
    pub reels: bool,
    pub stories: bool,
    pub stories_user: bool,
    pub tagged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_videos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_user_media_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_only: Option<bool>,
    /// Quando `true`, o sync deste source ignora a parada incremental e
    /// re-percorre o feed inteiro (útil quando o perfil reexpõe mídias antigas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_scan: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_profile: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_update_user_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_update_user_information: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_image_from_video: Option<InstagramExtractImageFromVideoSections>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_extracted_image_into_video_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_text: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_text_posts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_story_media_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_special_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id_hint: Option<String>,
    /// Handles que o perfil já teve (renames detectados ou nome legado do
    /// SCrawler). Usado para que a busca encontre o perfil pelo nome antigo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_handles: Option<Vec<String>>,
}

impl Default for InstagramSourceSyncOptions {
    fn default() -> Self {
        Self {
            timeline: true,
            reels: false,
            stories: false,
            stories_user: false,
            tagged: false,
            temporary: Some(false),
            favorite: Some(false),
            download_images: Some(true),
            download_videos: Some(true),
            get_user_media_only: Some(false),
            missing_only: Some(false),
            full_scan: Some(false),
            date_from: Some(String::new()),
            date_to: Some(String::new()),
            verified_profile: Some(true),
            force_update_user_name: Some(true),
            force_update_user_information: Some(false),
            extract_image_from_video: Some(InstagramExtractImageFromVideoSections::default()),
            place_extracted_image_into_video_folder: Some(false),
            download_text: Some(false),
            download_text_posts: Some(false),
            target_story_media_id: None,
            text_special_folder: Some(true),
            special_path: Some(String::new()),
            username_override: Some(String::new()),
            script_enabled: Some(false),
            script: Some(String::new()),
            description: Some(String::new()),
            color: Some(String::new()),
            user_id_hint: None,
            previous_handles: None,
        }
    }
}

/// Opções de sync do X/Twitter, espelhando o contrato do módulo Twitter do
/// SCrawler legado. Os modelos de download são combináveis (Media + Profile no
/// MVP); os timers seguem a semântica legada: `-1` desabilita o sleep e `-2`
/// (before-first) reutiliza o valor do sleep timer.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TwitterSourceSyncOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_use_graphql_endpoint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_use_graphql_endpoint: Option<bool>,
    /// Permite tweets de terceiros (retweets) no modelo media; espelho do
    /// MediaModelAllowNonUserTweets do SCrawler.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_non_user_tweets: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abort_on_limit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_already_parsed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_timer_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_timer_before_first_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_videos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_gifs: Option<bool>,
    /// Vídeos vão para uma subpasta `Video` (espelho do SeparateVideoFolder do
    /// SCrawler).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separate_video_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gifs_special_folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gifs_prefix: Option<String>,
    /// Compara o conteúdo (sha256) e descarta duplicatas idênticas; espelho do
    /// UseMD5Comparison do SCrawler.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_md5_comparison: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Id numérico do usuário no X (estável a renames). Metadado interno
    /// preenchido no import do SCrawler e preservado entre upserts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id_hint: Option<String>,
}

pub fn default_twitter_source_sync_options() -> TwitterSourceSyncOptions {
    TwitterSourceSyncOptions {
        media_model: Some(true),
        // A timeline completa e um backfill separado, nunca parte do sync normal.
        profile_model: Some(false),
        // Modelos opcionais (mais propensos a limite) ficam desligados.
        search_model: Some(false),
        likes_model: Some(false),
        search_use_graphql_endpoint: Some(true),
        profile_use_graphql_endpoint: Some(true),
        allow_non_user_tweets: Some(false),
        abort_on_limit: Some(true),
        download_already_parsed: Some(true),
        // SCrawler default: sleep desabilitado (TimerDisabled = -1).
        sleep_timer_secs: Some(-1),
        // SCrawler default: usa o valor do sleep timer (TimerFirstUseTheSame = -2).
        sleep_timer_before_first_secs: Some(-2),
        download_images: Some(true),
        download_videos: Some(true),
        download_gifs: Some(true),
        separate_video_folder: Some(true),
        gifs_special_folder: Some(String::new()),
        gifs_prefix: Some("GIF_".to_string()),
        use_md5_comparison: Some(false),
        temporary: Some(false),
        special_path: Some(String::new()),
        description: Some(String::new()),
        color: Some(String::new()),
        user_id_hint: None,
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TikTokSourceSyncOptions {
    /// Seções (espelho dos GetTimeline/GetStoriesUser/GetReposts do SCrawler).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_timeline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_stories_user: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_reposts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_liked_videos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liked_videos_limit: Option<i64>,
    /// Encerra a listagem depois de páginas consecutivas já baixadas, mas
    /// somente quando uma varredura completa anterior estabeleceu a baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liked_videos_incremental: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liked_videos_known_page_threshold: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collect_media_stats: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_existing_media_stats: Option<bool>,
    /// Story capturado pelo Companion: baixa só este vídeo na pasta Stories/ do
    /// perfil, sem enumerar a timeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_video_url: Option<String>,
    /// Vídeos baixam via yt-dlp; fotos (posts de slideshow) via gallery-dl.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_videos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_photos: Option<bool>,
    /// Usa o título nativo do vídeo no nome do arquivo em vez do id do post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_native_title: Option<bool>,
    /// Acrescenta o id do vídeo ao título nativo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_video_id_to_title: Option<bool>,
    /// Remove hashtags do título nativo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_tags_from_title: Option<bool>,
    /// Nomeia os arquivos no padrão do 4K Tokkit (`<handle>_<unix>_<id>`), para
    /// manter consistência com mídia importada. Tem precedência sobre o título.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokkit_file_naming: Option<bool>,
    /// Ajusta a data do arquivo para a data do post (yt-dlp --mtime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_parsed_video_date: Option<bool>,
    /// Baixa apenas posts criados a partir desta data (unix seconds). Espelha o
    /// range de download do 4K Tokkit. A data do post é derivada do id
    /// (`id >> 32` = unix). `None`/`0` = sem limite inferior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_from_date: Option<i64>,
    /// Baixa apenas posts criados até esta data (unix seconds). `None`/`0` = sem
    /// limite superior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_to_date: Option<i64>,
    /// Vídeos vão para uma subpasta `Video` (SeparateVideoFolder; default false
    /// no TikTok do SCrawler).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separate_video_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abort_on_limit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_timer_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Id estável do usuário no TikTok (uploader_id). Preenchido no sync e
    /// preservado entre upserts para detectar renames/duplicatas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id_hint: Option<String>,
}

pub fn default_tiktok_source_sync_options() -> TikTokSourceSyncOptions {
    TikTokSourceSyncOptions {
        get_timeline: Some(true),
        get_stories_user: Some(false),
        get_reposts: Some(false),
        get_liked_videos: Some(false),
        liked_videos_limit: Some(100),
        liked_videos_incremental: Some(true),
        liked_videos_known_page_threshold: Some(3),
        collect_media_stats: Some(true),
        refresh_existing_media_stats: Some(false),
        target_video_url: None,
        download_videos: Some(true),
        download_photos: Some(true),
        use_native_title: Some(false),
        add_video_id_to_title: Some(true),
        remove_tags_from_title: Some(false),
        tokkit_file_naming: Some(false),
        use_parsed_video_date: Some(true),
        download_from_date: None,
        download_to_date: None,
        separate_video_folder: Some(false),
        abort_on_limit: Some(true),
        sleep_timer_secs: Some(-1),
        temporary: Some(false),
        special_path: Some(String::new()),
        description: Some(String::new()),
        color: Some(String::new()),
        user_id_hint: None,
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram: Option<InstagramSourceSyncOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<TwitterSourceSyncOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiktok: Option<TikTokSourceSyncOptions>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    pub id: String,
    pub provider: String,
    pub source_kind: String,
    pub handle: String,
    pub display_name: String,
    pub account_id: Option<String>,
    pub group_id: Option<String>,
    pub labels: Vec<String>,
    pub ready_for_download: bool,
    pub sync_options: SourceSyncOptions,
    pub profile_image_path: Option<String>,
    pub profile_image_custom: bool,
    pub remote_state: String,
    pub is_subscription: bool,
    pub last_synced_at: Option<String>,
    pub sync_problem_code: Option<String>,
    pub sync_problem_message: Option<String>,
    pub sync_problem_at: Option<String>,
    pub created_at: Option<String>,
    pub importer_id: Option<String>,
    pub imported_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSourceProfilePatch {
    pub source_ids: Vec<String>,
    pub labels_to_add: Vec<String>,
    pub labels_to_remove: Vec<String>,
    pub ready_for_download: Option<bool>,
    pub sync_options_patch: Option<BatchSourceSyncOptionsPatch>,
    pub set_group_id: Option<Option<String>>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchSourceSyncOptionsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instagram: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiktok: Option<serde_json::Value>,
}

impl BatchSourceSyncOptionsPatch {
    pub fn for_provider(&self, provider: &str) -> Option<&serde_json::Value> {
        match provider {
            "instagram" => self.instagram.as_ref(),
            "twitter" => self.twitter.as_ref(),
            "tiktok" => self.tiktok.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSyncRun {
    pub id: String,
    pub source_id: String,
    pub account_id: String,
    pub provider: String,
    pub tool: String,
    pub trigger: String,
    pub status: String,
    pub summary: String,
    pub command_preview: String,
    pub manifest_summary_json: Option<String>,
    pub degraded_capabilities: Vec<String>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSyncRun {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub sync_scope: String,
    pub tool: String,
    pub trigger: String,
    pub status: String,
    pub summary: String,
    pub command_preview: String,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub id: String,
    pub scheduler_set_id: String,
    pub name: String,
    pub enabled: bool,
    pub mode: String,
    pub interval_minutes: u32,
    pub startup_delay_minutes: u32,
    pub notification_mode: String,
    pub target_filter: String,
    pub sort_index: i64,
    pub paused: bool,
    pub pause_mode: String,
    pub pause_until: Option<String>,
    pub skip_until: Option<String>,
    pub last_run_at: Option<String>,
    pub last_run_status: String,
    pub last_run_summary: Option<String>,
    pub next_due_at: Option<String>,
    pub notifications: SchedulerPlanNotifications,
    pub criteria: SchedulerPlanCriteria,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerSet {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub plans: Vec<SyncPlan>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerGroup {
    pub id: String,
    pub name: String,
    pub sort_index: i64,
    pub criteria: SchedulerPlanCriteria,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanRun {
    pub id: String,
    pub plan_id: String,
    pub scheduler_set_id: String,
    pub trigger: String,
    pub status: String,
    pub summary: String,
    pub source_count: u32,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSetting {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorRuntimeStatus {
    pub key: String,
    pub display_name: String,
    pub management_mode: String,
    pub active_version: Option<String>,
    pub bundled_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub status: String,
    pub last_checked_at: Option<String>,
    pub last_error: Option<String>,
    pub pending_version: Option<String>,
    pub progress_percent: Option<u32>,
    pub progress_detail: Option<String>,
    pub active_path: Option<String>,
    pub custom_path: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopRuntimeState {
    pub close_to_tray: bool,
    pub silent_mode: bool,
    pub tray_available: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub workspace_root: String,
    pub db_path: String,
    pub media_root: String,
    pub provider_catalog: Vec<ProviderDescriptor>,
    pub accounts: Vec<ProviderAccount>,
    pub account_sessions: Vec<ProviderAccountSession>,
    pub sources: Vec<SourceProfile>,
    pub source_sync_runs: Vec<SourceSyncRun>,
    pub account_sync_runs: Vec<AccountSyncRun>,
    pub scheduler_sets: Vec<SchedulerSet>,
    pub scheduler_groups: Vec<SchedulerGroup>,
    pub sync_plan_runs: Vec<SyncPlanRun>,
    pub app_settings: Vec<AppSetting>,
    pub connector_runtimes: Vec<ConnectorRuntimeStatus>,
    pub desktop_runtime: DesktopRuntimeState,
    /// Path absoluto de salvamento de mídia resolvido por perfil (source_id -> path).
    /// Permite à UI filtrar e exibir onde cada perfil grava sem recomputar a lógica.
    #[serde(default)]
    pub source_media_paths: HashMap<String, String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountUpsert {
    pub id: Option<String>,
    pub provider: String,
    pub display_name: String,
    pub auth_mode: String,
    pub auth_state: String,
    pub capabilities: Vec<String>,
    pub last_validated_at: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountCookieImport {
    pub account_id: String,
    pub import_format: String,
    pub content: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogQuery {
    pub limit: Option<u32>,
    pub level: Option<String>,
    pub scope: Option<String>,
    pub provider: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDebugQuery {
    pub limit: Option<u32>,
    pub provider: Option<String>,
    pub source_id: Option<String>,
    pub event_type: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfileUpsert {
    pub id: Option<String>,
    pub provider: String,
    pub source_kind: String,
    pub handle: String,
    pub display_name: String,
    pub account_id: Option<String>,
    pub group_id: Option<String>,
    pub labels: Vec<String>,
    pub ready_for_download: bool,
    pub sync_options: SourceSyncOptions,
    pub remote_state: Option<String>,
    pub is_subscription: Option<bool>,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceProfileDeleteMode {
    UserOnly,
    WithMedia,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfileDeleteInput {
    pub id: String,
    pub mode: SourceProfileDeleteMode,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSourceSyncInput {
    pub id: String,
    pub trigger: Option<String>,
    pub run_mode: Option<String>,
    pub sync_options_override: Option<SourceSyncOptions>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSourceAvailabilityInput {
    pub source_ids: Vec<String>,
    pub account_id_override: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAvailabilityCheckItem {
    pub source_id: String,
    pub provider: String,
    pub previous_handle: String,
    pub current_handle: Option<String>,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAvailabilityCheckResult {
    pub snapshot: WorkspaceSnapshot,
    pub requested: u32,
    pub processed: u32,
    pub unchanged: u32,
    pub updated_handle: u32,
    pub marked_problem: u32,
    pub skipped: u32,
    pub failed: u32,
    pub items: Vec<SourceAvailabilityCheckItem>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerSetUpsert {
    pub id: Option<String>,
    pub name: String,
    pub active: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerGroupUpsert {
    pub id: Option<String>,
    pub name: String,
    pub sort_index: Option<i64>,
    #[serde(default)]
    pub criteria: SchedulerPlanCriteria,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanUpsert {
    pub id: Option<String>,
    pub scheduler_set_id: String,
    pub name: String,
    pub enabled: bool,
    pub mode: String,
    pub interval_minutes: u32,
    pub startup_delay_minutes: u32,
    pub notification_mode: String,
    pub target_filter: String,
    pub sort_index: Option<i64>,
    pub pause_mode: Option<String>,
    pub pause_until: Option<String>,
    #[serde(default)]
    pub notifications: SchedulerPlanNotifications,
    #[serde(default)]
    pub criteria: SchedulerPlanCriteria,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanTargetPreviewInput {
    pub scheduler_set_id: Option<String>,
    pub plan_id: Option<String>,
    pub criteria: SchedulerPlanCriteria,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanTargetPreviewSource {
    pub id: String,
    pub handle: String,
    pub provider: String,
    pub labels: Vec<String>,
    pub ready_for_download: bool,
    pub remote_state: String,
    pub subscription: bool,
    pub last_synced_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanTargetPreview {
    pub source_count: u32,
    pub sources: Vec<SyncPlanTargetPreviewSource>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSyncPlanPauseInput {
    pub id: String,
    pub pause_mode: String,
    pub pause_until: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSyncPlanNowInput {
    pub id: String,
    pub force: Option<bool>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipSyncPlanInput {
    pub id: String,
    pub mode: String,
    pub minutes: Option<u32>,
    pub until: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveSyncPlanInput {
    pub id: String,
    pub direction: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneSyncPlanInput {
    pub id: String,
}

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerPlanNotifications {
    pub enabled: bool,
    pub simple: bool,
    pub show_image: bool,
    pub show_user_icon: bool,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerPlanCriteria {
    pub regular: bool,
    pub temporary: bool,
    pub favorite: bool,
    pub ready_for_download: bool,
    pub ignore_ready_for_download: bool,
    pub download_users: bool,
    pub download_subscriptions: bool,
    pub user_exists: bool,
    pub user_suspended: bool,
    pub user_deleted: bool,
    pub labels_no: bool,
    #[serde(default)]
    pub labels_included: Vec<String>,
    #[serde(default)]
    pub labels_excluded: Vec<String>,
    pub ignore_excluded_labels: bool,
    #[serde(default)]
    pub sites_included: Vec<String>,
    #[serde(default)]
    pub sites_excluded: Vec<String>,
    #[serde(default)]
    pub group_ids_included: Vec<String>,
    #[serde(default)]
    pub group_ids_excluded: Vec<String>,
    pub users_count: Option<u32>,
    pub days_number: Option<u32>,
    pub days_is_downloaded: bool,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    #[serde(default = "default_date_in_range")]
    pub date_in_range: bool,
    pub advanced_expression: Option<String>,
}

fn default_date_in_range() -> bool {
    true
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingUpsert {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppBuildChannel {
    Release,
    Development,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppBuildInfo {
    pub version: String,
    pub commit_sha: String,
    pub dirty: bool,
    pub channel: AppBuildChannel,
    pub display_version: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateStatus {
    pub build: AppBuildInfo,
    pub latest_version: String,
    pub release_url: String,
    pub published_at: Option<String>,
    pub update_available: bool,
}
