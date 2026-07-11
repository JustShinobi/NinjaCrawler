use super::*;

fn cookie(name: &str, value: &str) -> ProviderAccountCookie {
    ProviderAccountCookie {
        domain: ".x.com".to_string(),
        name: name.to_string(),
        value: value.to_string(),
        path: "/".to_string(),
        expires_at: None,
        secure: true,
        http_only: true,
    }
}

fn capture(token: &str) -> CompanionAccountCapture {
    CompanionAccountCapture {
        provider: "twitter".to_string(),
        current_url: "https://x.com/home".to_string(),
        identity: crate::domain::models::CompanionAccountIdentity {
            provider_user_id: Some("42".to_string()),
            username: "ninja".to_string(),
        },
        cookies: vec![
            cookie("auth_token", token),
            cookie("ct0", &format!("csrf-{token}")),
        ],
        authorization: HashMap::from([("userAgent".to_string(), "Mozilla/5.0 Test".to_string())]),
    }
}

#[test]
fn import_and_revert_swap_the_protected_session() {
    let temp = tempfile::tempdir().expect("temp dir");
    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            ProviderAccountUpsert {
                id: Some("account-1".to_string()),
                provider: "twitter".to_string(),
                display_name: "Existing".to_string(),
                auth_mode: "imported_session".to_string(),
                auth_state: "ready".to_string(),
                capabilities: vec!["posts".to_string()],
                last_validated_at: None,
            },
        )?;
        save_provider_account_cookies_with_connection(
            connection,
            test_layout,
            "account-1",
            vec![cookie("auth_token", "old"), cookie("ct0", "old-csrf")],
        )?;
        let result = import_companion_account_with_connection(
            connection,
            test_layout,
            CompanionAccountImportInput {
                capture: capture("new"),
                target_account_id: Some("account-1".to_string()),
                create_display_name: None,
            },
        )?;
        assert!(result.can_revert);
        assert!(load_provider_account_cookies_with_connection(
            connection,
            test_layout,
            "account-1"
        )?
        .iter()
        .any(|item| item.name == "auth_token" && item.value == "new"));
        let plaintext = connection
            .query_row(
                "SELECT COUNT(*) FROM provider_account_settings
                 WHERE account_id='account-1' AND setting_key='twitter.auth.userAgent'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(plaintext, 0);
        revert_provider_account_import_with_connection(connection, test_layout, "account-1")?;
        assert!(load_provider_account_cookies_with_connection(
            connection,
            test_layout,
            "account-1"
        )?
        .iter()
        .any(|item| item.name == "auth_token" && item.value == "old"));
        Ok(())
    })
    .expect("import and revert");
}

#[test]
fn client_hints_are_kept_only_for_instagram() {
    let mut capture = capture("token");
    capture.authorization.insert(
        "secChUa".to_string(),
        "\"Chromium\";v=\"130\"".to_string(),
    );
    capture
        .authorization
        .insert("secChUaPlatformVersion".to_string(), "\"15.0.0\"".to_string());

    let twitter = companion_metadata("twitter", &capture);
    assert_eq!(twitter.user_agent.as_deref(), Some("Mozilla/5.0 Test"));
    assert!(twitter.sec_ch_ua.is_none());
    assert!(twitter.sec_ch_ua_platform_version.is_none());

    let instagram = companion_metadata("instagram", &capture);
    assert_eq!(instagram.user_agent.as_deref(), Some("Mozilla/5.0 Test"));
    assert_eq!(instagram.sec_ch_ua.as_deref(), Some("\"Chromium\";v=\"130\""));
    assert_eq!(
        instagram.sec_ch_ua_platform_version.as_deref(),
        Some("\"15.0.0\"")
    );
}

#[test]
fn preview_is_redacted() {
    let temp = tempfile::tempdir().expect("temp dir");
    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        let preview = preview_companion_account_with_connection(
            connection,
            test_layout,
            &capture("super-secret"),
        )?;
        assert_eq!(preview.cookie_count, 2);
        assert!(!serde_json::to_string(&preview)
            .map_err(|error| error.to_string())?
            .contains("super-secret"));
        let mut reddit = capture("token");
        reddit.provider = "reddit".to_string();
        assert!(
            preview_companion_account_with_connection(connection, test_layout, &reddit,).is_err()
        );
        Ok(())
    })
    .expect("preview");
}
