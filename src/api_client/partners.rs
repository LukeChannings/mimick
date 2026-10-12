use super::{ImmichApiClient, UserSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartnerDirection {
    SharedBy,
    SharedWith,
}

fn partners_path(direction: PartnerDirection) -> &'static str {
    match direction {
        PartnerDirection::SharedBy => "/api/partners?direction=shared-by",
        PartnerDirection::SharedWith => "/api/partners?direction=shared-with",
    }
}

impl ImmichApiClient {
    pub async fn fetch_partners(
        &self,
        direction: PartnerDirection,
    ) -> Result<Vec<UserSummary>, String> {
        let resp = self
            .send_json(reqwest::Method::GET, partners_path(direction), None)
            .await?;
        let mut partners: Vec<UserSummary> = resp.json().await.map_err(|err| err.to_string())?;
        partners.sort_by_key(|p| p.name.to_lowercase());
        Ok(partners)
    }

    pub async fn add_partner(&self, user_id: &str) -> Result<(), String> {
        let body = serde_json::json!({ "sharedWithId": user_id });
        self.send_json(reqwest::Method::POST, "/api/partners", Some(body))
            .await
            .map(|_| ())
    }

    pub async fn remove_partner(&self, user_id: &str) -> Result<(), String> {
        let path = format!("/api/partners/{user_id}");
        self.send_json(reqwest::Method::DELETE, &path, None)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partners_path_names_the_direction() {
        assert!(partners_path(PartnerDirection::SharedBy).ends_with("shared-by"));
        assert!(partners_path(PartnerDirection::SharedWith).ends_with("shared-with"));
    }

    #[test]
    fn partner_json_reads_as_a_user() {
        let partner: UserSummary = serde_json::from_value(serde_json::json!({
            "id": "u2",
            "name": "Ana",
            "email": "ana@example.com",
            "inTimeline": true,
            "avatarColor": "primary",
            "profileImagePath": "",
            "profileChangedAt": "2024-01-01T00:00:00.000Z"
        }))
        .unwrap();
        assert_eq!(partner.email, "ana@example.com");
    }
}
