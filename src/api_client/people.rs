use super::ImmichApiClient;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Option<String>>,
}

impl ImmichApiClient {
    pub async fn update_person(
        &self,
        person_id: &str,
        update: &PersonUpdate,
    ) -> Result<(), String> {
        let body = serde_json::to_value(update).map_err(|err| err.to_string())?;
        let path = format!("/api/people/{person_id}");
        self.send_json(reqwest::Method::PUT, &path, Some(body))
            .await
            .map(|_| ())
    }

    pub async fn merge_people(&self, into_id: &str, merge_ids: &[String]) -> Result<(), String> {
        let path = format!("/api/people/{into_id}/merge");
        let body = serde_json::json!({ "ids": merge_ids });
        self.send_json(reqwest::Method::POST, &path, Some(body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_sends_only_set_fields() {
        let rename = PersonUpdate {
            name: Some("Ana".into()),
            ..PersonUpdate::default()
        };
        assert_eq!(
            serde_json::to_value(&rename).unwrap(),
            serde_json::json!({ "name": "Ana" })
        );
    }

    #[test]
    fn clearing_a_birthday_sends_null() {
        let clear = PersonUpdate {
            birth_date: Some(None),
            ..PersonUpdate::default()
        };
        assert_eq!(
            serde_json::to_value(&clear).unwrap(),
            serde_json::json!({ "birthDate": null })
        );
    }

    #[test]
    fn person_json_reads_birth_date() {
        let person: super::super::Person = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "Ana",
            "isHidden": true,
            "birthDate": "1990-05-04"
        }))
        .unwrap();
        assert!(person.is_hidden);
        assert_eq!(person.birth_date.as_deref(), Some("1990-05-04"));
    }
}
