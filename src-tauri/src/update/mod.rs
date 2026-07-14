use serde::{Deserialize, Serialize};

use crate::session::store::Store;

const RELEASES_URL: &str = "https://api.github.com/repos/tybadev/tyba-terminal/releases/latest";
const CHECK_INTERVAL_SECS: i64 = 6 * 60 * 60;
const MAX_BODY_BYTES: usize = 256 * 1024;
const TIMEOUT_SECS: u64 = 5;

const KEY_LAST_CHECK: &str = "update.last_check";
const KEY_LATEST: &str = "update.latest";
const KEY_DISMISSED: &str = "update.dismissed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateStatus {
    pub info: UpdateInfo,
    pub dismissed: bool,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
}

pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('v').unwrap_or(trimmed);
    if stripped.contains('-') || stripped.contains('+') {
        return None;
    }
    let mut parts = stripped.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

fn release_to_info(release: GhRelease, current: &str) -> Option<UpdateInfo> {
    if release.draft || release.prerelease {
        return None;
    }
    if !is_newer(&release.tag_name, current) {
        return None;
    }
    let version = release
        .tag_name
        .trim()
        .strip_prefix('v')
        .unwrap_or(release.tag_name.trim())
        .to_string();
    Some(UpdateInfo {
        version,
        url: release.html_url,
        published_at: release.published_at,
    })
}

pub fn pick_update(body: &str, current: &str) -> Option<UpdateInfo> {
    let release: GhRelease = serde_json::from_str(body).ok()?;
    release_to_info(release, current)
}

pub fn should_check(now: i64, last_check: Option<i64>) -> bool {
    match last_check {
        Some(last) => now < last || now - last >= CHECK_INTERVAL_SECS,
        None => true,
    }
}

fn cached_status(store: &Store, current: &str) -> Option<UpdateStatus> {
    let raw = store.get_setting(KEY_LATEST).ok()??;
    let info: UpdateInfo = serde_json::from_str(&raw).ok()?;
    if !is_newer(&info.version, current) {
        return None;
    }
    let dismissed = store
        .get_setting(KEY_DISMISSED)
        .ok()
        .flatten()
        .is_some_and(|v| v == info.version);
    Some(UpdateStatus { info, dismissed })
}

async fn fetch_latest(current: &str) -> Option<UpdateInfo> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .user_agent(format!("tyba/{current}"))
        .build()
        .ok()?;
    let response = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).ok()?;
    pick_update(&text, current)
}

pub async fn check(store: &Store, current: &str, now: i64) -> Option<UpdateStatus> {
    let last_check = store
        .get_setting(KEY_LAST_CHECK)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok());

    if !should_check(now, last_check) {
        return cached_status(store, current);
    }

    let _ = store.set_setting(KEY_LAST_CHECK, &now.to_string());

    match fetch_latest(current).await {
        Some(info) => {
            if let Ok(json) = serde_json::to_string(&info) {
                let _ = store.set_setting(KEY_LATEST, &json);
            }
            let dismissed = store
                .get_setting(KEY_DISMISSED)
                .ok()
                .flatten()
                .is_some_and(|v| v == info.version);
            Some(UpdateStatus { info, dismissed })
        }
        None => cached_status(store, current),
    }
}

pub fn dismiss(store: &Store, version: &str) -> Result<(), String> {
    store
        .set_setting(KEY_DISMISSED, version)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tag_with_and_without_v() {
        assert_eq!(parse_version("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version("0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version(" v1.20.3 "), Some((1, 20, 3)));
    }

    #[test]
    fn refuses_what_is_not_semver_estrito() {
        assert_eq!(parse_version("0.1"), None);
        assert_eq!(parse_version("0.1.2.3"), None);
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("v0.1.2-rc1"), None);
        assert_eq!(parse_version("0.1.2+build"), None);
    }

    #[test]
    fn compares_by_number_not_by_string() {
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.99"));
        assert!(is_newer("v1.0.0", "0.99.99"));
    }

    #[test]
    fn never_pushes_the_user_backwards() {
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
    }

    #[test]
    fn unparseable_version_never_notifies() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("v0.1.1", "sei-la"));
        assert!(!is_newer("v0.2.0-rc1", "0.1.0"));
    }

    fn release_json(tag: &str, draft: bool, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/tybadev/tyba-terminal/releases/tag/{tag}","draft":{draft},"prerelease":{prerelease},"published_at":"2026-07-20T10:00:00Z"}}"#
        )
    }

    #[test]
    fn picks_a_newer_stable_release() {
        let info = pick_update(&release_json("v0.1.2", false, false), "0.1.0").unwrap();
        assert_eq!(info.version, "0.1.2");
        assert!(info.url.starts_with("https://github.com/tybadev/"));
        assert_eq!(info.published_at.as_deref(), Some("2026-07-20T10:00:00Z"));
    }

    #[test]
    fn ignores_draft_and_prerelease() {
        assert!(pick_update(&release_json("v0.2.0", true, false), "0.1.0").is_none());
        assert!(pick_update(&release_json("v0.2.0", false, true), "0.1.0").is_none());
    }

    #[test]
    fn ignores_the_same_version_we_are_running() {
        assert!(pick_update(&release_json("v0.1.0", false, false), "0.1.0").is_none());
    }

    #[test]
    fn garbage_body_never_panics() {
        assert!(pick_update("", "0.1.0").is_none());
        assert!(pick_update("null", "0.1.0").is_none());
        assert!(pick_update(r#"{"message":"Not Found"}"#, "0.1.0").is_none());
        assert!(pick_update("<html>502</html>", "0.1.0").is_none());
    }

    #[test]
    fn cache_window_holds_for_six_hours() {
        let now = 1_000_000;
        assert!(should_check(now, None));
        assert!(!should_check(now, Some(now)));
        assert!(!should_check(now, Some(now - CHECK_INTERVAL_SECS + 1)));
        assert!(should_check(now, Some(now - CHECK_INTERVAL_SECS)));
    }

    #[test]
    fn clock_walking_backwards_does_not_freeze_the_check() {
        let now = 1_000_000;
        assert!(should_check(now, Some(now + 99_999)));
    }

    #[tokio::test]
    #[ignore = "usa rede: prova que o TLS do rustls sobe de verdade"]
    async fn tls_handshake_reaches_the_github_api() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .user_agent("tyba/0.1.0")
            .build()
            .expect("cliente https não construiu");
        let response = client
            .get(RELEASES_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .expect("handshake TLS falhou — provider ou raízes ausentes");
        assert!(
            response.status().as_u16() < 500,
            "resposta inesperada do GitHub: {}",
            response.status()
        );
    }
}
