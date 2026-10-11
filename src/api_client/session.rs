//! Login sessions and the Locked Folder: Immich only unlocks it for a signed-in
//! session, never for an API key. The token lives in memory and is never saved.

use reqwest::Method;
use reqwest::header::HeaderMap;
use std::time::Duration;

use super::{ImmichApiClient, MetadataSearchFilters, UserSummary, header_map};

/// The parts of `POST /api/auth/login` Mimick needs.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogin {
    pub access_token: String,
    pub user_id: String,
}

/// `GET /api/auth/status`: whether the account has a password and a PIN, and the unlock state.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthStatus {
    pub pin_code: bool,
    pub password: bool,
    pub is_elevated: bool,
    pub pin_expires_at: Option<String>,
}

impl MetadataSearchFilters {
    /// Filters that match only Locked Folder assets (needs an unlocked session).
    pub fn locked() -> Self {
        Self {
            visibility: Some("locked".to_string()),
            ..Self::default()
        }
    }
}

/// Body for `PUT /api/assets`, moving assets into (`"locked"`) or out of (`"timeline"`) the folder.
fn visibility_body(asset_ids: &[String], visibility: &str) -> serde_json::Value {
    serde_json::json!({ "ids": asset_ids, "visibility": visibility })
}

fn login_body(email: &str, password: &str) -> serde_json::Value {
    serde_json::json!({ "email": email, "password": password })
}

fn pin_body(pin: &str) -> serde_json::Value {
    serde_json::json!({ "pinCode": pin })
}

/// Immich's `message` from an error body (such as "Incorrect email or password"), else the status.
fn error_message(status: reqwest::StatusCode, body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            json.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("HTTP {status}"))
}

impl ImmichApiClient {
    async fn send_auth_request(
        &self,
        method: Method,
        path: &str,
        auth: HeaderMap,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let mut request = self
            .client
            .request(method, format!("{}{}", base_url, path))
            .headers(auth)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(15));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let resp = request.send().await.map_err(|err| err.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(error_message(status, &body));
        }
        Ok(resp)
    }

    /// Sign in with email and password. Doesn't activate the session; see `set_session_token`.
    pub async fn login(&self, email: &str, password: &str) -> Result<SessionLogin, String> {
        let body = login_body(email, password);
        let resp = self
            .send_auth_request(
                Method::POST,
                "/api/auth/login",
                HeaderMap::new(),
                Some(body),
            )
            .await?;
        resp.json().await.map_err(|err| err.to_string())
    }

    /// Route every request through `token` (Bearer), or back to the API key with `None`.
    pub fn set_session_token(&self, token: Option<String>) {
        *self.session_token.write() = token;
    }

    pub fn has_session(&self) -> bool {
        self.session_token.read().is_some()
    }

    /// Auth status for the current credentials (the session when set, else the API key).
    pub async fn auth_status(&self) -> Result<AuthStatus, String> {
        let resp = self
            .send_auth_request(Method::GET, "/api/auth/status", self.auth_headers(), None)
            .await?;
        resp.json().await.map_err(|err| err.to_string())
    }

    /// The account the current credentials belong to (`GET /api/users/me`).
    pub async fn fetch_current_user(&self) -> Result<UserSummary, String> {
        let resp = self
            .send_auth_request(Method::GET, "/api/users/me", self.auth_headers(), None)
            .await?;
        resp.json().await.map_err(|err| err.to_string())
    }

    /// Set the account's first PIN code (needs an active session).
    pub async fn setup_pin(&self, pin: &str) -> Result<(), String> {
        let body = pin_body(pin);
        let path = "/api/auth/pin-code";
        self.send_auth_request(Method::POST, path, self.auth_headers(), Some(body))
            .await
            .map(drop)
    }

    /// Unlock the Locked Folder for the active session with the account PIN.
    pub async fn unlock_session(&self, pin: &str) -> Result<(), String> {
        let body = pin_body(pin);
        let path = "/api/auth/session/unlock";
        self.send_auth_request(Method::POST, path, self.auth_headers(), Some(body))
            .await
            .map(drop)
    }

    /// Sign `token` out, which also ends its unlock. Call after `set_session_token(None)`.
    pub async fn logout_session(&self, token: &str) -> Result<(), String> {
        let auth = header_map("", Some(token));
        self.send_auth_request(Method::POST, "/api/auth/logout", auth, None)
            .await
            .map(drop)
    }

    /// Move assets into or out of the Locked Folder (needs an unlocked session).
    pub async fn set_assets_visibility(
        &self,
        asset_ids: &[String],
        visibility: &str,
    ) -> Result<(), String> {
        let body = visibility_body(asset_ids, visibility);
        self.send_auth_request(Method::PUT, "/api/assets", self.auth_headers(), Some(body))
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_pin_bodies_use_immich_field_names() {
        assert_eq!(
            login_body("a@b.c", "pw"),
            serde_json::json!({ "email": "a@b.c", "password": "pw" })
        );
        assert_eq!(
            pin_body("123456"),
            serde_json::json!({ "pinCode": "123456" })
        );
    }

    #[test]
    fn locked_filters_and_visibility_body() {
        let json = serde_json::to_value(MetadataSearchFilters::locked()).unwrap();
        assert_eq!(json, serde_json::json!({ "visibility": "locked" }));
        assert_eq!(
            visibility_body(&["a".to_string()], "timeline"),
            serde_json::json!({ "ids": ["a"], "visibility": "timeline" })
        );
    }

    #[test]
    fn session_login_and_status_deserialize() {
        let login: SessionLogin = serde_json::from_value(serde_json::json!({
            "accessToken": "tok", "userId": "u1", "userEmail": "a@b.c", "isAdmin": false,
        }))
        .unwrap();
        assert_eq!(login.access_token, "tok");
        assert_eq!(login.user_id, "u1");

        let status: AuthStatus = serde_json::from_value(serde_json::json!({
            "pinCode": true, "password": true, "isElevated": true,
            "pinExpiresAt": "2026-10-11T10:15:00.000Z",
        }))
        .unwrap();
        assert!(status.pin_code && status.is_elevated);
        assert_eq!(
            status.pin_expires_at.as_deref(),
            Some("2026-10-11T10:15:00.000Z")
        );
        let minimal: AuthStatus = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(minimal, AuthStatus::default());
    }

    #[test]
    fn error_message_prefers_immich_message() {
        let status = reqwest::StatusCode::UNAUTHORIZED;
        let body = r#"{"message":"Incorrect email or password","statusCode":401}"#;
        assert_eq!(error_message(status, body), "Incorrect email or password");
        assert_eq!(error_message(status, "<html>"), "HTTP 401 Unauthorized");
    }
}
