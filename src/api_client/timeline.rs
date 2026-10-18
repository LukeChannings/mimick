//! Timeline bucket API: the Main library's month-sectioned, stack-collapsed listing.
//!
//! `GET /api/timeline/buckets` returns per-month counts; `GET /api/timeline/bucket`
//! returns one month's assets in a **columnar** layout (one array per field). Both
//! require Immich >= 1.133 (see [`supports_timeline_buckets`]). See ADR 0001.
//!
//! Consumed by the virtual grid in later phases; nothing in the UI calls this yet.
#![allow(dead_code)]

use std::time::Duration;

use serde_json::Value;

use super::errors::{RequestContext, classify_http_issue, classify_network_issue};
use super::{ImmichApiClient, ServerAbout};

/// First Immich release serving the columnar timeline bucket format.
const MIN_TIMELINE_VERSION: (u64, u64, u64) = (1, 133, 0);

/// One month of the Main library as announced by `/timeline/buckets`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct TimeBucket {
    /// Opaque server key (an ISO date such as `2025-06-01...`); echo it back verbatim.
    #[serde(rename = "timeBucket")]
    pub time_bucket: String,
    /// Number of tiles (stacks count once) in this bucket.
    pub count: u32,
}

impl TimeBucket {
    /// Display label such as "June 2025".
    pub fn label(&self) -> String {
        bucket_label(&self.time_bucket)
    }
}

/// Thin per-asset row transposed from a columnar bucket response.
#[derive(Debug, Clone, PartialEq)]
pub struct TileAsset {
    pub id: String,
    /// Width / height.
    pub ratio: f64,
    pub thumbhash: Option<String>,
    pub is_image: bool,
    /// Video duration as served (`"0:00:05.000000"`); `None` for images.
    pub duration: Option<String>,
    pub is_favorite: bool,
    pub visibility: String,
    pub file_created_at: String,
    /// `(stack id, member count)` when this tile stands in for a stack.
    pub stack: Option<(String, u32)>,
}

/// Whether the server is new enough for the timeline bucket API (>= 1.133.0).
/// An unparseable version fails open: the endpoint call itself will error loudly.
pub fn supports_timeline_buckets(about: &ServerAbout) -> bool {
    match parse_semver(&about.version) {
        Some(version) => version >= MIN_TIMELINE_VERSION,
        None => true,
    }
}

/// Parse `major.minor.patch` from strings like `v1.133.0` or `1.133.0-rc.1`.
fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// "June 2025" from an opaque bucket key; falls back to the key itself.
pub fn bucket_label(bucket: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let year = bucket
        .get(0..4)
        .filter(|y| y.bytes().all(|b| b.is_ascii_digit()));
    let month = bucket
        .get(5..7)
        .and_then(|m| m.parse::<usize>().ok())
        .filter(|m| (1..=12).contains(m));
    match (year, month) {
        (Some(year), Some(month)) if bucket.as_bytes().get(4) == Some(&b'-') => {
            format!("{} {}", MONTHS[month - 1], year)
        }
        _ => bucket.to_string(),
    }
}

/// Fetch one column as a vector of values; `required` columns must exist and be arrays.
fn column<'a>(
    obj: &'a serde_json::Map<String, Value>,
    name: &str,
    required: bool,
) -> Result<Option<&'a Vec<Value>>, String> {
    match obj.get(name) {
        Some(Value::Array(items)) => Ok(Some(items)),
        Some(Value::Null) | None if !required => Ok(None),
        Some(Value::Null) | None => Err(format!("timeline bucket is missing column '{name}'")),
        Some(_) => Err(format!("timeline bucket column '{name}' is not an array")),
    }
}

/// Parse a `[stackId, assetCount]` tuple; the count arrives as a string.
fn parse_stack_tuple(value: &Value) -> Result<Option<(String, u32)>, String> {
    let items = match value {
        Value::Null => return Ok(None),
        Value::Array(items) => items,
        _ => return Err("stack entry is not an array".to_string()),
    };
    let [id, count] = items.as_slice() else {
        return Err(format!(
            "stack entry has {} elements, expected 2",
            items.len()
        ));
    };
    let id = id
        .as_str()
        .ok_or_else(|| "stack id is not a string".to_string())?;
    let count = match count {
        Value::String(s) => s.trim().parse::<u32>().ok(),
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
    .ok_or_else(|| format!("invalid stack asset count: {count}"))?;
    Ok(Some((id.to_string(), count)))
}

/// Transpose a columnar `/timeline/bucket` response into row structs.
///
/// Mismatched column lengths, missing required columns, or malformed cells are an
/// `Err`, never a panic. `thumbhash`, `duration` and `stack` are optional columns
/// (absent or null means "no value for any row").
pub fn transpose_bucket(body: &Value) -> Result<Vec<TileAsset>, String> {
    let obj = body
        .as_object()
        .ok_or_else(|| "timeline bucket response is not an object".to_string())?;

    let ids = column(obj, "id", true)?.expect("required column");
    let len = ids.len();
    let sized = |name: &str, required: bool| -> Result<Option<&Vec<Value>>, String> {
        let col = column(obj, name, required)?;
        if let Some(col) = col
            && col.len() != len
        {
            return Err(format!(
                "timeline bucket column '{name}' has {} entries, expected {len}",
                col.len()
            ));
        }
        Ok(col)
    };
    let ratio = sized("ratio", true)?.expect("required column");
    let is_favorite = sized("isFavorite", true)?.expect("required column");
    let visibility = sized("visibility", true)?.expect("required column");
    let is_image = sized("isImage", true)?.expect("required column");
    let file_created_at = sized("fileCreatedAt", true)?.expect("required column");
    let thumbhash = sized("thumbhash", false)?;
    let duration = sized("duration", false)?;
    let stack = sized("stack", false)?;

    let text = |value: &Value, name: &str, row: usize| -> Result<String, String> {
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("timeline bucket '{name}' at row {row} is not a string"))
    };
    let flag = |value: &Value, name: &str, row: usize| -> Result<bool, String> {
        value
            .as_bool()
            .ok_or_else(|| format!("timeline bucket '{name}' at row {row} is not a boolean"))
    };
    let opt_text = |col: Option<&Vec<Value>>, row: usize| -> Option<String> {
        col.and_then(|c| c[row].as_str()).map(str::to_string)
    };

    let mut tiles = Vec::with_capacity(len);
    for row in 0..len {
        tiles.push(TileAsset {
            id: text(&ids[row], "id", row)?,
            ratio: ratio[row]
                .as_f64()
                .ok_or_else(|| format!("timeline bucket 'ratio' at row {row} is not a number"))?,
            thumbhash: opt_text(thumbhash, row),
            is_image: flag(&is_image[row], "isImage", row)?,
            duration: opt_text(duration, row),
            is_favorite: flag(&is_favorite[row], "isFavorite", row)?,
            visibility: text(&visibility[row], "visibility", row)?,
            file_created_at: text(&file_created_at[row], "fileCreatedAt", row)?,
            stack: match stack {
                Some(col) => parse_stack_tuple(&col[row])?,
                None => None,
            },
        });
    }
    Ok(tiles)
}

impl ImmichApiClient {
    /// Per-month counts for the Main library, newest first.
    pub async fn fetch_timeline_buckets(
        &self,
        visibility: &str,
    ) -> Result<Vec<TimeBucket>, String> {
        let body = self
            .get_timeline_json(
                "/api/timeline/buckets",
                &[
                    ("visibility", visibility),
                    ("withStacked", "true"),
                    ("order", "desc"),
                ],
            )
            .await?;
        serde_json::from_value(body).map_err(|err| err.to_string())
    }

    /// Contents of one bucket, stacks collapsed to their primary.
    pub async fn fetch_timeline_bucket(
        &self,
        bucket: &str,
        visibility: &str,
    ) -> Result<Vec<TileAsset>, String> {
        let body = self
            .get_timeline_json(
                "/api/timeline/bucket",
                &[
                    ("timeBucket", bucket),
                    ("visibility", visibility),
                    ("withStacked", "true"),
                    ("order", "desc"),
                ],
            )
            .await?;
        transpose_bucket(&body)
    }

    async fn get_timeline_json(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let mut url =
            url::Url::parse(&format!("{}{}", base_url, path)).map_err(|err| err.to_string())?;
        url.query_pairs_mut().extend_pairs(params.iter().copied());

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
                let body = resp.json::<Value>().await.map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(body)
            }
            Ok(resp) => {
                self.set_issue(classify_http_issue(
                    RequestContext::Timeline,
                    resp.status().as_u16(),
                    None,
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(err) => {
                *self.active_url.lock().await = None;
                self.set_issue(classify_network_issue(RequestContext::Timeline, &err))
                    .await;
                Err(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn about(version: &str) -> ServerAbout {
        ServerAbout {
            version: version.to_string(),
            ..ServerAbout::default()
        }
    }

    #[test]
    fn version_gate_requires_1_133() {
        assert!(!supports_timeline_buckets(&about("1.132.3")));
        assert!(!supports_timeline_buckets(&about("v1.99.0")));
        assert!(!supports_timeline_buckets(&about("0.200.0")));
        assert!(supports_timeline_buckets(&about("1.133.0")));
        assert!(supports_timeline_buckets(&about("v1.133.1")));
        assert!(supports_timeline_buckets(&about("1.140.2")));
        assert!(supports_timeline_buckets(&about("2.0.0")));
        assert!(supports_timeline_buckets(&about("1.133.0-rc.1")));
    }

    #[test]
    fn version_gate_fails_open_on_garbage() {
        assert!(supports_timeline_buckets(&about("")));
        assert!(supports_timeline_buckets(&about("nightly")));
        assert!(supports_timeline_buckets(&about("1.133")));
        assert!(supports_timeline_buckets(&ServerAbout::default()));
    }

    #[test]
    fn bucket_deserializes_and_labels() {
        let buckets: Vec<TimeBucket> = serde_json::from_value(json!([
            { "timeBucket": "2025-06-01T00:00:00.000Z", "count": 12 },
            { "timeBucket": "2025-05-01", "count": 3 },
        ]))
        .unwrap();
        assert_eq!(buckets[0].count, 12);
        assert_eq!(buckets[0].label(), "June 2025");
        assert_eq!(buckets[1].label(), "May 2025");
        assert_eq!(bucket_label("garbage"), "garbage");
        assert_eq!(bucket_label("2025-13-01"), "2025-13-01");
        assert_eq!(bucket_label("é"), "é");
    }

    fn sample() -> Value {
        json!({
            "id": ["a", "b", "c"],
            "ownerId": ["u", "u", "u"],
            "ratio": [1.5, 0.75, 1.0],
            "isFavorite": [false, true, false],
            "visibility": ["timeline", "timeline", "timeline"],
            "isTrashed": [false, false, false],
            "isImage": [true, false, true],
            "thumbhash": ["hash", null, "h2"],
            "fileCreatedAt": ["2025-06-03T10:00:00.000Z", "2025-06-02T10:00:00.000Z", "2025-06-01T10:00:00.000Z"],
            "localOffsetHours": [0, 0, 0],
            "duration": [null, "0:00:05.000000", null],
            "stack": [null, null, ["stack-1", "4"]],
            "projectionType": [null, null, null],
            "livePhotoVideoId": [null, null, null],
            "city": [null, null, null],
            "country": [null, null, null],
            "latitude": [null, null, null],
            "longitude": [null, null, null]
        })
    }

    #[test]
    fn transposes_columns_into_rows() {
        let tiles = transpose_bucket(&sample()).unwrap();
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[0].id, "a");
        assert_eq!(tiles[0].ratio, 1.5);
        assert_eq!(tiles[0].thumbhash.as_deref(), Some("hash"));
        assert!(tiles[0].is_image);
        assert_eq!(tiles[0].stack, None);
        assert!(tiles[1].is_favorite);
        assert!(!tiles[1].is_image);
        assert_eq!(tiles[1].thumbhash, None);
        assert_eq!(tiles[1].duration.as_deref(), Some("0:00:05.000000"));
        assert_eq!(tiles[2].stack, Some(("stack-1".to_string(), 4)));
        assert_eq!(tiles[2].file_created_at, "2025-06-01T10:00:00.000Z");
    }

    #[test]
    fn empty_bucket_is_empty() {
        let body = json!({
            "id": [], "ratio": [], "isFavorite": [], "visibility": [],
            "isImage": [], "fileCreatedAt": []
        });
        assert!(transpose_bucket(&body).unwrap().is_empty());
    }

    #[test]
    fn short_column_is_an_error() {
        let mut body = sample();
        body["ratio"] = json!([1.0]);
        assert!(transpose_bucket(&body).unwrap_err().contains("ratio"));

        let mut body = sample();
        body["stack"] = json!([null]);
        assert!(transpose_bucket(&body).unwrap_err().contains("stack"));
    }

    #[test]
    fn null_or_missing_required_column_is_an_error() {
        let mut body = sample();
        body["isImage"] = Value::Null;
        assert!(transpose_bucket(&body).is_err());

        let mut body = sample();
        body.as_object_mut().unwrap().remove("fileCreatedAt");
        assert!(transpose_bucket(&body).is_err());

        assert!(transpose_bucket(&json!([])).is_err());
    }

    #[test]
    fn null_cell_in_required_column_is_an_error() {
        let mut body = sample();
        body["ratio"] = json!([1.5, null, 1.0]);
        assert!(transpose_bucket(&body).is_err());
    }

    #[test]
    fn optional_columns_may_be_absent() {
        let mut body = sample();
        let obj = body.as_object_mut().unwrap();
        obj.remove("stack");
        obj.remove("thumbhash");
        obj.remove("duration");
        let tiles = transpose_bucket(&body).unwrap();
        assert!(
            tiles
                .iter()
                .all(|t| t.stack.is_none() && t.thumbhash.is_none())
        );
    }

    #[test]
    fn stack_tuple_parsing() {
        assert_eq!(parse_stack_tuple(&Value::Null), Ok(None));
        assert_eq!(
            parse_stack_tuple(&json!(["s", "12"])),
            Ok(Some(("s".to_string(), 12)))
        );
        assert_eq!(
            parse_stack_tuple(&json!(["s", 2])),
            Ok(Some(("s".to_string(), 2)))
        );
        assert!(parse_stack_tuple(&json!(["s", "many"])).is_err());
        assert!(parse_stack_tuple(&json!(["s"])).is_err());
        assert!(parse_stack_tuple(&json!(["s", "2", "x"])).is_err());
        assert!(parse_stack_tuple(&json!([1, "2"])).is_err());
        assert!(parse_stack_tuple(&json!("s")).is_err());
        assert!(parse_stack_tuple(&json!(["s", "-1"])).is_err());
    }

    #[test]
    fn malformed_stack_cell_is_an_error() {
        let mut body = sample();
        body["stack"] = json!([null, null, ["stack-1", "x"]]);
        assert!(transpose_bucket(&body).is_err());
    }
}
