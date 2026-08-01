//! Resolve o `secUid` de um perfil do TikTok via WebView autenticado.
//!
//! O yt-dlp precisa do `secUid` para enumerar a timeline de um perfil. Ele o
//! procura no HTML da página do perfil — que hoje só devolve o desafio do WAF —
//! e, como fallback, na embed page. Perfis com o embed desabilitado fazem as
//! duas rotas falharem, e o yt-dlp aborta com "Unable to extract secondary user
//! ID" sem enumerar nada.
//!
//! Aqui abrimos a página num WebView real e autenticado (o mesmo caminho já
//! usado pelos likes), que passa pelo WAF. A própria SPA do perfil busca a
//! timeline ao carregar, e essas requisições carregam o `secUid` na query
//! string: lê-lo dali é mais confiável que raspar o HTML (que este tipo de
//! perfil renderiza no cliente) ou chamar `api/user/detail` (que responde 200
//! com corpo vazio). Com o `secUid` em mãos, o sync volta ao yt-dlp usando
//! `tiktokuser:<secUid>`, e todo o pipeline de download segue inalterado.

use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, WebviewWindow, Wry};

use crate::infrastructure::{connector_debug, tiktok_likes_runtime};

const WINDOW_LABEL_PREFIX: &str = "tiktok-secuid";
const PAGE_READY_TIMEOUT: Duration = Duration::from_secs(45);
/// A página do perfil ainda precisa passar pelo desafio do WAF antes de montar
/// a timeline, então damos mais folga que no home.
const PROFILE_LOAD_TIMEOUT: Duration = Duration::from_secs(90);
/// Depois de a página abrir, a SPA ainda leva um instante para pedir a
/// timeline — é essa requisição que carrega o `secUid`.
const SEC_UID_TIMEOUT: Duration = Duration::from_secs(60);

/// Motivo pelo qual o `secUid` não pôde ser resolvido. Diferenciado do erro
/// genérico porque o chamador reporta cada caso de forma distinta.
pub enum TikTokSecUidError {
    /// O perfil não existe mais (ou foi banido): nem a conta autenticada o
    /// resolve.
    ProfileUnavailable(String),
    /// Qualquer outra falha (rede, sessão expirada, WebView indisponível).
    Failed(String),
}

impl TikTokSecUidError {
    pub fn message(&self) -> &str {
        match self {
            Self::ProfileUnavailable(message) | Self::Failed(message) => message,
        }
    }
}

/// Resolve o `secUid` de `handle` usando a sessão autenticada de `account_id`.
pub fn resolve_sec_uid<C>(
    app: &AppHandle,
    account_id: &str,
    handle: &str,
    is_cancelled: C,
) -> Result<String, TikTokSecUidError>
where
    C: Fn() -> bool,
{
    let handle = handle.trim().trim_start_matches('@').to_string();
    if handle.is_empty() {
        return Err(TikTokSecUidError::Failed(
            "Cannot resolve a TikTok secUid without a handle.".to_string(),
        ));
    }
    let session = tiktok_likes_runtime::load_webview_session(account_id)
        .map_err(TikTokSecUidError::Failed)?;

    connector_debug::append_current(
        "internal.tiktok",
        "system",
        "secuid.resolve.begin",
        format!("account_id={account_id}\nhandle={handle}"),
    );

    let window = tiktok_likes_runtime::open_tiktok_window(
        app,
        format!("{WINDOW_LABEL_PREFIX}-{account_id}"),
        format!("TikTok profile — @{handle}"),
        session.user_agent.clone(),
        session.cookies.clone(),
    )
    .map_err(TikTokSecUidError::Failed)?;

    let outcome = run_resolution(&window, &handle, &is_cancelled);
    let _ = window.close();

    match &outcome {
        Ok(sec_uid) => connector_debug::append_current(
            "internal.tiktok",
            "response",
            "secuid.resolve.complete",
            format!("handle={handle}\nsec_uid={sec_uid}"),
        ),
        Err(error) => connector_debug::append_current(
            "internal.tiktok",
            "error",
            "secuid.resolve.failed",
            format!("handle={handle}\nerror={}", error.message()),
        ),
    }
    outcome
}

fn run_resolution<C>(
    window: &WebviewWindow<Wry>,
    handle: &str,
    is_cancelled: &C,
) -> Result<String, TikTokSecUidError>
where
    C: Fn() -> bool,
{
    tiktok_likes_runtime::wait_until(
        window,
        "document.readyState === \"complete\"",
        PAGE_READY_TIMEOUT,
        |value| value.as_bool().unwrap_or(false),
        "The TikTok WebView did not finish loading.",
        is_cancelled,
    )
    .map_err(TikTokSecUidError::Failed)?;

    // Sem os cookies de sessão o WebView vê o TikTok deslogado, e um perfil
    // acessível só para seguidores não resolveria.
    if let Ok(url) = tiktok_likes_runtime::TIKTOK_HOME.parse() {
        if let Ok(cookies) = window.cookies_for_url(url) {
            let has_session = cookies.iter().any(|cookie| {
                matches!(
                    cookie.name(),
                    "sessionid" | "sessionid_ss" | "sid_tt" | "sid_guard"
                )
            });
            if !has_session {
                return Err(TikTokSecUidError::Failed(
                    "The TikTok WebView did not retain the authenticated cookies.".to_string(),
                ));
            }
        }
    }

    navigate_to_profile(window, handle, is_cancelled)?;

    // O `secUid` aparece na query string das chamadas que a própria página faz
    // para montar a timeline.
    match tiktok_likes_runtime::wait_until(
        window,
        SEC_UID_SCRIPT,
        SEC_UID_TIMEOUT,
        |value| value.as_str().is_some_and(is_plausible_sec_uid),
        "TikTok did not expose the profile secUid.",
        is_cancelled,
    ) {
        Ok(value) => Ok(value
            .as_str()
            .unwrap_or_default()
            .to_string()),
        Err(error) => {
            let state = tiktok_likes_runtime::evaluate_json(window, PROFILE_DIAGNOSTIC_SCRIPT)
                .unwrap_or_else(Value::String);
            connector_debug::append_current(
                "internal.tiktok",
                "error",
                "secuid.page_state",
                format!("handle={handle}\nerror={error}\nstate={state}"),
            );
            Err(TikTokSecUidError::Failed(format!(
                "{error} Page state: {state}"
            )))
        }
    }
}

/// Navega até a página do perfil e espera o WAF liberar. O critério é o
/// `<title>`: a página de desafio não tem título de perfil. Se o TikTok
/// redirecionar para fora do perfil, a conta não existe mais.
fn navigate_to_profile<C>(
    window: &WebviewWindow<Wry>,
    handle: &str,
    is_cancelled: &C,
) -> Result<(), TikTokSecUidError>
where
    C: Fn() -> bool,
{
    let profile_url = format!("https://www.tiktok.com/@{handle}")
        .parse()
        .map_err(|error| TikTokSecUidError::Failed(format!("Invalid TikTok URL: {error}")))?;
    window.navigate(profile_url).map_err(|error| {
        TikTokSecUidError::Failed(format!("Could not open the TikTok profile page: {error}"))
    })?;

    let lowercase_handle = handle.to_ascii_lowercase();
    let script = format!(
        "(document.readyState === \"complete\") && document.title.toLowerCase().includes(\"@{lowercase_handle}\")"
    );
    if let Err(error) = tiktok_likes_runtime::wait_until(
        window,
        &script,
        PROFILE_LOAD_TIMEOUT,
        |value| value.as_bool().unwrap_or(false),
        "TikTok did not load the profile page within the timeout.",
        is_cancelled,
    ) {
        let state = tiktok_likes_runtime::evaluate_json(window, PROFILE_DIAGNOSTIC_SCRIPT)
            .unwrap_or_else(Value::String);
        connector_debug::append_current(
            "internal.tiktok",
            "error",
            "secuid.page_state",
            format!("handle={handle}\nerror={error}\nstate={state}"),
        );
        // Redirecionado para fora do perfil: a conta não existe mais.
        let redirected_away = state
            .get("href")
            .and_then(Value::as_str)
            .is_some_and(|href| !href.to_ascii_lowercase().contains(&lowercase_handle));
        let message = format!("{error} Page state: {state}");
        return Err(if redirected_away {
            TikTokSecUidError::ProfileUnavailable(message)
        } else {
            TikTokSecUidError::Failed(message)
        });
    }
    Ok(())
}

/// O `secUid` tem sempre o prefixo `MS4wLjABAAAA`; o yt-dlp só aceita a forma
/// longa (76 caracteres) na rota `tiktokuser:`.
fn is_plausible_sec_uid(value: &str) -> bool {
    value.len() >= 40 && value.starts_with("MS4wLjABAAAA")
}

/// Procura o `secUid` nas requisições que a página já fez.
const SEC_UID_SCRIPT: &str = r#"(() => {
  for (const entry of performance.getEntriesByType("resource")) {
    const match = /[?&]secUid=([^&]+)/.exec(entry.name ?? "");
    if (match) {
      const decoded = decodeURIComponent(match[1]);
      if (decoded.startsWith("MS4wLjABAAAA")) {
        return decoded;
      }
    }
  }
  return "";
})()"#;

/// Estado da página usado só para depurar por que o secUid não apareceu.
const PROFILE_DIAGNOSTIC_SCRIPT: &str = r#"({
  href: location.href,
  title: document.title,
  readyState: document.readyState,
  resourceCount: performance.getEntriesByType("resource").length,
  bodyStart: (document.body?.innerText ?? "").slice(0, 200)
})"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_uid_shape_is_validated() {
        // Forma longa, a única que a rota `tiktokuser:` do yt-dlp aceita.
        assert!(is_plausible_sec_uid(
            "MS4wLjABAAAAkOSMQ0c6rPAl3tg6OErMfQiOaHWcAK9ix96HcIMo8LFzD6Qxhi4yARWxfa9Zi5pi"
        ));
        assert!(!is_plausible_sec_uid("MS4wLjABAAAAshort"));
        assert!(!is_plausible_sec_uid(""));
        assert!(!is_plausible_sec_uid(
            "NOTASECUID000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn sec_uid_script_reads_the_requests_the_page_already_made() {
        assert!(SEC_UID_SCRIPT.contains("getEntriesByType(\"resource\")"));
        assert!(SEC_UID_SCRIPT.contains("secUid="));
        assert!(SEC_UID_SCRIPT.contains("MS4wLjABAAAA"));
    }

    #[test]
    fn diagnostic_script_reports_what_the_page_actually_loaded() {
        assert!(PROFILE_DIAGNOSTIC_SCRIPT.contains("href"));
        assert!(PROFILE_DIAGNOSTIC_SCRIPT.contains("resourceCount"));
        assert!(PROFILE_DIAGNOSTIC_SCRIPT.contains("bodyStart"));
    }
}
