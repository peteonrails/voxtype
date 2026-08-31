//! SuperGrok / X Premium+ device-code login. Tokens stay in Voxtype's data dir.
//!
//! Client id is grok-cli's public id (xAI has no third-party CLI registration).

use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
/// Must stay well below typical `expires_in` (~3600s).
const REFRESH_SKEW_SECS: u64 = 120;

static REFRESH_LOCK: Mutex<()> = Mutex::new(());
static TOKEN_CACHE: Mutex<Option<(String, u64)>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthTokens {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAuth {
    version: u32,
    tokens: OAuthTokens,
    #[serde(default)]
    token_endpoint: String,
    #[serde(default)]
    last_refresh_unix: u64,
}

pub fn store_path() -> PathBuf {
    if let Some(base) = std::env::var_os("VOXTYPE_DATA_DIR") {
        return PathBuf::from(base).join("xai-oauth.json");
    }
    Config::data_dir().join("xai-oauth.json")
}

fn validate_oauth_url(url: &str) -> Result<()> {
    let ok = (url.starts_with("https://auth.x.ai/") || url.starts_with("https://accounts.x.ai/"))
        && !url.contains(' ')
        && url.is_ascii();
    if !ok {
        bail!(
            "refusing xAI OAuth URL (expected https://auth.x.ai/ or https://accounts.x.ai/): {url}"
        );
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn token_exp_unix(auth: &StoredAuth) -> u64 {
    let lifetime = auth.tokens.expires_in.unwrap_or(15 * 60);
    auth.last_refresh_unix.saturating_add(lifetime)
}

#[derive(Debug, PartialEq, Eq)]
enum RefreshPlan {
    UseStore,
    Redeem,
    LoggedOut,
}

fn plan_refresh(caller: &StoredAuth, store: Option<&StoredAuth>, force: bool) -> RefreshPlan {
    let Some(fresh) = store else {
        return RefreshPlan::LoggedOut;
    };
    if fresh.tokens.refresh_token != caller.tokens.refresh_token
        || fresh.tokens.access_token != caller.tokens.access_token
    {
        return RefreshPlan::UseStore;
    }
    if !force && !needs_refresh_auth(fresh) {
        return RefreshPlan::UseStore;
    }
    RefreshPlan::Redeem
}

fn needs_refresh_auth(auth: &StoredAuth) -> bool {
    now_unix() + REFRESH_SKEW_SECS >= token_exp_unix(auth)
}

fn read_store() -> Result<Option<StoredAuth>> {
    let path = store_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let auth: StoredAuth = serde_json::from_str(&raw).context("parse xai-oauth.json")?;
    Ok(Some(auth))
}

fn write_store(auth: &StoredAuth) -> Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(auth)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn cache_put(token: &str, exp_unix: u64) {
    if let Ok(mut g) = TOKEN_CACHE.lock() {
        *g = Some((token.to_string(), exp_unix));
    }
}

fn cache_get_if_fresh() -> Option<String> {
    let g = TOKEN_CACHE.lock().ok()?;
    let (tok, exp) = g.as_ref()?;
    if now_unix() + REFRESH_SKEW_SECS >= *exp {
        return None;
    }
    Some(tok.clone())
}

pub fn is_logged_in() -> bool {
    read_store().ok().flatten().is_some()
}

pub fn logout() -> Result<()> {
    let path = store_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    if let Ok(mut g) = TOKEN_CACHE.lock() {
        *g = None;
    }
    Ok(())
}

pub fn status_line() -> String {
    match read_store() {
        Ok(Some(_)) => format!("xAI OAuth: signed in ({})", store_path().display()),
        Ok(None) => "xAI OAuth: not signed in (run: voxtype setup xai --login)".into(),
        Err(e) => format!("xAI OAuth: error reading store: {e}"),
    }
}

fn discovery() -> Result<String> {
    let resp: serde_json::Value = ureq::get(DISCOVERY_URL)
        .timeout(Duration::from_secs(20))
        .call()
        .context("xAI OIDC discovery")?
        .into_json()
        .context("discovery JSON")?;
    let token = resp
        .get("token_endpoint")
        .and_then(|v| v.as_str())
        .context("missing token_endpoint")?
        .to_string();
    validate_oauth_url(&token)?;
    Ok(token)
}

fn refresh_tokens(auth: &StoredAuth, force: bool) -> Result<StoredAuth> {
    let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let store = read_store()?;
    match plan_refresh(auth, store.as_ref(), force) {
        RefreshPlan::LoggedOut => {
            bail!("xAI OAuth session gone (logged out?) — run: voxtype setup xai --login")
        }
        RefreshPlan::UseStore => {
            return Ok(store.expect("UseStore implies Some"));
        }
        RefreshPlan::Redeem => {}
    }
    let auth = store.unwrap_or_else(|| auth.clone());
    let token_endpoint = if !auth.token_endpoint.is_empty() {
        validate_oauth_url(&auth.token_endpoint)?;
        auth.token_endpoint.clone()
    } else {
        discovery()?
    };
    let resp = ureq::post(&token_endpoint)
        .timeout(Duration::from_secs(30))
        .set("Accept", "application/json")
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", auth.tokens.refresh_token.as_str()),
        ])
        .context("xAI token refresh")?;
    let v: serde_json::Value = resp.into_json().context("refresh JSON")?;
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if access.is_empty() {
        bail!("xAI refresh missing access_token — run: voxtype setup xai --login");
    }
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| auth.tokens.refresh_token.clone());
    let mut next = auth.clone();
    next.tokens.access_token = access;
    next.tokens.refresh_token = refresh;
    next.tokens.expires_in = v.get("expires_in").and_then(|x| x.as_u64());
    next.token_endpoint = token_endpoint;
    next.last_refresh_unix = now_unix();
    write_store(&next)?;
    Ok(next)
}

pub fn access_token() -> Result<String> {
    if let Some(tok) = cache_get_if_fresh() {
        return Ok(tok);
    }
    let auth = read_store()?.context("no xAI OAuth session — run: voxtype setup xai --login")?;
    if !needs_refresh_auth(&auth) {
        cache_put(&auth.tokens.access_token, token_exp_unix(&auth));
        return Ok(auth.tokens.access_token);
    }
    tracing::info!("xAI OAuth access token near expiry; refreshing");
    let next = refresh_tokens(&auth, false)?;
    cache_put(&next.tokens.access_token, token_exp_unix(&next));
    Ok(next.tokens.access_token)
}

pub fn force_refresh() -> Result<String> {
    if let Ok(mut g) = TOKEN_CACHE.lock() {
        *g = None;
    }
    let auth = read_store()?.context("no xAI OAuth session")?;
    let next = refresh_tokens(&auth, true)?;
    cache_put(&next.tokens.access_token, token_exp_unix(&next));
    Ok(next.tokens.access_token)
}

pub fn login_device_code(open_browser: bool) -> Result<()> {
    let token_endpoint = discovery()?;
    let resp = ureq::post(DEVICE_CODE_URL)
        .timeout(Duration::from_secs(20))
        .set("Accept", "application/json")
        .send_form(&[("client_id", CLIENT_ID), ("scope", SCOPE)])
        .context("xAI device-code request")?;
    let device: serde_json::Value = resp.into_json().context("device-code JSON")?;
    let device_code = device
        .get("device_code")
        .and_then(|x| x.as_str())
        .context("missing device_code")?
        .to_string();
    let user_code = device
        .get("user_code")
        .and_then(|x| x.as_str())
        .unwrap_or("?");
    let verification = device
        .get("verification_uri_complete")
        .and_then(|x| x.as_str())
        .or_else(|| device.get("verification_uri").and_then(|x| x.as_str()))
        .unwrap_or("https://accounts.x.ai/oauth2/device");
    let expires_in = device
        .get("expires_in")
        .and_then(|x| x.as_u64())
        .unwrap_or(1800);
    let mut interval = device
        .get("interval")
        .and_then(|x| x.as_u64())
        .unwrap_or(5)
        .max(1);

    validate_oauth_url(verification)?;
    eprintln!();
    eprintln!("Sign in with SuperGrok or X Premium+:");
    eprintln!("  1. Open: {verification}");
    eprintln!("  2. If prompted, enter code: {user_code}");
    if open_browser {
        let _ = std::process::Command::new("xdg-open")
            .arg(verification)
            .spawn();
        eprintln!("  (Tried to open a browser)");
    }
    eprintln!("Waiting for approval (polling every {interval}s, up to {expires_in}s)...");

    let deadline = Instant::now() + Duration::from_secs(expires_in);
    loop {
        if Instant::now() >= deadline {
            bail!("Timed out waiting for xAI device authorization");
        }
        std::thread::sleep(Duration::from_secs(interval));
        let poll = ureq::post(&token_endpoint)
            .timeout(Duration::from_secs(30))
            .set("Accept", "application/json")
            .send_form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", device_code.as_str()),
            ]);
        match poll {
            Ok(resp) => {
                let v: serde_json::Value = resp.into_json().context("token JSON")?;
                let access = v
                    .get("access_token")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let refresh = v
                    .get("refresh_token")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if access.is_empty() || refresh.is_empty() {
                    bail!("xAI token response missing access/refresh token");
                }
                let auth = StoredAuth {
                    version: 1,
                    tokens: OAuthTokens {
                        access_token: access.clone(),
                        refresh_token: refresh,
                        expires_in: v.get("expires_in").and_then(|x| x.as_u64()),
                    },
                    token_endpoint,
                    last_refresh_unix: now_unix(),
                };
                write_store(&auth)?;
                cache_put(&access, token_exp_unix(&auth));
                eprintln!("Signed in. Tokens stored at {}", store_path().display());
                return Ok(());
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if body.contains("authorization_pending") {
                    continue;
                }
                if body.contains("slow_down") {
                    interval += 5;
                    continue;
                }
                bail!("xAI device token poll HTTP {code}: {body}");
            }
            Err(e) => bail!("xAI device token poll failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn store_path_honors_voxtype_data_dir() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("VOXTYPE_DATA_DIR");
        std::env::set_var("VOXTYPE_DATA_DIR", "/tmp/voxtype-xai-test-data");
        let p = store_path();
        match prev {
            Some(v) => std::env::set_var("VOXTYPE_DATA_DIR", v),
            None => std::env::remove_var("VOXTYPE_DATA_DIR"),
        }
        assert!(p.ends_with("xai-oauth.json"));
        assert!(p.to_string_lossy().contains("voxtype-xai-test-data"));
    }

    #[test]
    fn validate_oauth_url_rejects_api_host() {
        assert!(validate_oauth_url("https://api.x.ai/v1/stt").is_err());
        assert!(validate_oauth_url("http://auth.x.ai/oauth2/token").is_err());
        assert!(validate_oauth_url("https://auth.x.ai/oauth2/token").is_ok());
    }

    fn sample(last_refresh_unix: u64, expires_in: u64) -> StoredAuth {
        StoredAuth {
            version: 1,
            tokens: OAuthTokens {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_in: Some(expires_in),
            },
            token_endpoint: "https://auth.x.ai/oauth2/token".into(),
            last_refresh_unix,
        }
    }

    #[test]
    fn hour_token_is_fresh_just_after_issue() {
        let auth = sample(now_unix(), 3600);
        assert!(!needs_refresh_auth(&auth));
    }

    #[test]
    fn token_shorter_than_skew_always_needs_refresh() {
        let auth = sample(now_unix(), 60);
        assert!(needs_refresh_auth(&auth));
    }

    #[test]
    fn cache_ttl_follows_expires_in_not_a_fixed_five_hours() {
        cache_put("dead", now_unix());
        assert!(cache_get_if_fresh().is_none());
        cache_put("live", now_unix() + 3600);
        assert_eq!(cache_get_if_fresh().as_deref(), Some("live"));
    }

    #[test]
    fn force_refresh_redeems_even_when_ttl_says_fresh() {
        let auth = sample(now_unix(), 3600);
        assert!(!needs_refresh_auth(&auth));
        assert_eq!(plan_refresh(&auth, Some(&auth), true), RefreshPlan::Redeem);
        assert_eq!(
            plan_refresh(&auth, Some(&auth), false),
            RefreshPlan::UseStore
        );
    }

    #[test]
    fn concurrent_rotation_does_not_redeem_stale_refresh() {
        let caller = sample(now_unix(), 3600);
        let mut store = caller.clone();
        store.tokens.access_token = "new-a".into();
        store.tokens.refresh_token = "new-r".into();
        assert_eq!(
            plan_refresh(&caller, Some(&store), true),
            RefreshPlan::UseStore
        );
    }

    #[test]
    fn logout_during_refresh_is_logged_out() {
        let auth = sample(now_unix(), 3600);
        assert_eq!(plan_refresh(&auth, None, true), RefreshPlan::LoggedOut);
    }
}
