//! Stacks: fetch a stack's members and probe whether the API key may read stacks.
//!
//! Immich exposes no endpoint listing an API key's permissions, so `stack.read`
//! is validated by probing (see `probe_permissions`). Consumed by later phases;
//! nothing in the UI calls this yet.
#![allow(dead_code)]

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use super::errors::{RequestContext, classify_http_issue, classify_network_issue};
use super::{ImmichApiClient, LibraryAsset};

/// Error string returned by `fetch_stack` when the key lacks `stack.read`.
pub const STACK_FORBIDDEN: &str = "HTTP 403 Forbidden";

/// Whether an error from `fetch_stack` means the key lacks `stack.read`.
pub fn is_forbidden(err: &str) -> bool {
    err == STACK_FORBIDDEN
}

/// A stack and its members (full asset DTOs; the primary comes first).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct StackResponse {
    pub id: String,
    #[serde(rename = "primaryAssetId")]
    pub primary_asset_id: String,
    pub assets: Vec<LibraryAsset>,
}

/// Outcome of probing a single API-key permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Missing,
    /// The probe was inconclusive (timeout, network, unexpected status). Do not warn.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionProbeResult {
    /// Immich permission name, e.g. `stack.read`.
    pub permission: &'static str,
    /// What the user loses without it, phrased for a settings warning.
    pub feature_hint: &'static str,
    pub state: PermissionState,
}

/// One row of the probe table. `path` may contain `{uuid}`, replaced by a random v4 UUID.
struct PermissionProbe {
    permission: &'static str,
    feature_hint: &'static str,
    path: &'static str,
}

const PERMISSION_PROBES: &[PermissionProbe] = &[PermissionProbe {
    permission: "stack.read",
    feature_hint: "Stacked photos will not show their other members in the viewer, and archiving or deleting a stack will be unavailable.",
    path: "/api/stacks?primaryAssetId={uuid}",
}];

/// Classify a probe's HTTP outcome: `None` means no response (timeout / network error).
fn classify_probe_status(status: Option<u16>) -> PermissionState {
    match status {
        Some(200) => PermissionState::Granted,
        Some(403) => PermissionState::Missing,
        _ => PermissionState::Unknown,
    }
}

/// Random UUID v4 string. Only needs to be unlikely to exist, so std's randomly
/// seeded hasher is enough and avoids a new dependency.
fn random_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let word = RandomState::new().build_hasher().finish();
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

impl ImmichApiClient {
    /// Members of a stack. A 403 (missing `stack.read`) returns `STACK_FORBIDDEN`;
    /// check with [`is_forbidden`].
    pub async fn fetch_stack(&self, id: &str) -> Result<StackResponse, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let mut url =
            url::Url::parse(&format!("{}/api/stacks/", base_url)).map_err(|err| err.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "Invalid server URL".to_string())?
            .pop()
            .push(id);

        match self
            .client
            .get(url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let stack = resp
                    .json::<StackResponse>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(stack)
            }
            // A missing permission is expected and handled by callers; not an API-key issue.
            Ok(resp) if resp.status().as_u16() == 403 => Err(STACK_FORBIDDEN.to_string()),
            Ok(resp) => {
                self.set_issue(classify_http_issue(
                    RequestContext::Stack,
                    resp.status().as_u16(),
                    None,
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(err) => {
                *self.active_url.lock().await = None;
                self.set_issue(classify_network_issue(RequestContext::Stack, &err))
                    .await;
                Err(err.to_string())
            }
        }
    }

    /// Probe each known optional API-key permission. Never records an `ApiIssue`.
    pub async fn probe_permissions(&self) -> Vec<PermissionProbeResult> {
        let mut results = Vec::with_capacity(PERMISSION_PROBES.len());
        for probe in PERMISSION_PROBES {
            let state = self
                .probe_status(&probe.path.replace("{uuid}", &random_uuid_v4()))
                .await;
            results.push(PermissionProbeResult {
                permission: probe.permission,
                feature_hint: probe.feature_hint,
                state,
            });
        }
        results
    }

    async fn probe_status(&self, path: &str) -> PermissionState {
        let Some(base_url) = self.get_active_url().await else {
            return PermissionState::Unknown;
        };
        let settings = self.settings_snapshot();
        let status = self
            .client
            .get(format!("{}{}", base_url, path))
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .ok()
            .map(|resp| resp.status().as_u16());
        classify_probe_status(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_classification() {
        assert_eq!(classify_probe_status(Some(200)), PermissionState::Granted);
        assert_eq!(classify_probe_status(Some(403)), PermissionState::Missing);
        // timeout / network failure
        assert_eq!(classify_probe_status(None), PermissionState::Unknown);
        for status in [204, 401, 404, 500, 503] {
            assert_eq!(
                classify_probe_status(Some(status)),
                PermissionState::Unknown
            );
        }
    }

    #[test]
    fn probe_table_lists_stack_read() {
        let row = &PERMISSION_PROBES[0];
        assert_eq!(row.permission, "stack.read");
        assert!(row.path.contains("{uuid}"));
        assert!(!row.feature_hint.is_empty());
    }

    #[test]
    fn random_uuid_is_v4_shaped_and_varies() {
        let a = random_uuid_v4();
        let b = random_uuid_v4();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }

    #[test]
    fn forbidden_error_is_recognised() {
        assert!(is_forbidden(STACK_FORBIDDEN));
        assert!(!is_forbidden("HTTP 404 Not Found"));
    }

    #[test]
    fn stack_response_reuses_asset_deserialization() {
        let stack: StackResponse = serde_json::from_value(serde_json::json!({
            "id": "s1",
            "primaryAssetId": "a1",
            "assets": [{
                "id": "a1",
                "originalFileName": "x.jpg",
                "originalMimeType": "image/jpeg",
                "fileCreatedAt": "2025-06-01T00:00:00.000Z",
                "type": "IMAGE",
                "thumbhash": null,
                "width": null,
                "height": null,
                "checksum": "da39a3ee5e6b4b0d3255bfef95601890afd80709"
            }]
        }))
        .unwrap();
        assert_eq!(stack.primary_asset_id, "a1");
        assert_eq!(stack.assets.len(), 1);
        assert_eq!(
            stack.assets[0].checksum.as_deref(),
            Some("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
    }
}
