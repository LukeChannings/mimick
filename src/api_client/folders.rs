use super::{ImmichApiClient, LibraryAsset};

fn folder_assets_path(path: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("path", path)
        .finish();
    format!("/api/view/folder?{query}")
}

impl ImmichApiClient {
    pub async fn fetch_folder_paths(&self) -> Result<Vec<String>, String> {
        let resp = self
            .send_json(reqwest::Method::GET, "/api/view/folder/unique-paths", None)
            .await?;
        resp.json().await.map_err(|err| err.to_string())
    }

    pub async fn fetch_folder_assets(&self, path: &str) -> Result<Vec<LibraryAsset>, String> {
        let resp = self
            .send_json(reqwest::Method::GET, &folder_assets_path(path), None)
            .await?;
        resp.json().await.map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_path_is_query_encoded() {
        assert_eq!(
            folder_assets_path("/photos/Trip & Co"),
            "/api/view/folder?path=%2Fphotos%2FTrip+%26+Co"
        );
    }
}
