//! Bucket selection policy for the masonry grid.
//!
//! Maps a `GridQuality` choice (auto / explicit) and a row's rendered height
//! to a concrete `ThumbnailSize`. `fallback_bucket` defines the degradation
//! chain used when a server lacks higher buckets (e.g. no fullsize).

use crate::api_client::ThumbnailSize;

/// Edge length (px) of Immich's `thumbnail` bucket. Rows rendered taller than
/// this in *device* pixels would upscale it visibly, so Auto switches to the
/// `preview` bucket beyond it.
pub(crate) const THUMBNAIL_EDGE: f32 = 250.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridQuality {
    Auto,
    #[default]
    Thumbnail,
    Preview,
    Fullsize,
}

impl GridQuality {
    pub fn parse(s: &str) -> Self {
        match s {
            "thumbnail" => Self::Thumbnail,
            "preview" => Self::Preview,
            "fullsize" => Self::Fullsize,
            _ => Self::Auto,
        }
    }
}

/// `device_h` is the row height in device pixels (logical height × the
/// widget's scale factor), so HiDPI screens get sharp tiles too.
pub(crate) fn bucket_for_row_height(device_h: f32, quality: GridQuality) -> ThumbnailSize {
    match quality {
        GridQuality::Thumbnail => ThumbnailSize::Thumbnail,
        GridQuality::Preview => ThumbnailSize::Preview,
        GridQuality::Fullsize => ThumbnailSize::Fullsize,
        GridQuality::Auto => {
            if device_h <= THUMBNAIL_EDGE {
                ThumbnailSize::Thumbnail
            } else {
                ThumbnailSize::Preview
            }
        }
    }
}

/// Smaller size to try when a requested bucket isn't available on the server.
pub(crate) fn fallback_bucket(size: ThumbnailSize) -> Option<ThumbnailSize> {
    match size {
        ThumbnailSize::Fullsize => Some(ThumbnailSize::Preview),
        ThumbnailSize::Preview | ThumbnailSize::Thumbnail => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_thumbnail_under_threshold() {
        assert!(matches!(
            bucket_for_row_height(200.0, GridQuality::Auto),
            ThumbnailSize::Thumbnail
        ));
        assert!(matches!(
            bucket_for_row_height(THUMBNAIL_EDGE, GridQuality::Auto),
            ThumbnailSize::Thumbnail
        ));
        assert!(matches!(
            bucket_for_row_height(THUMBNAIL_EDGE + 1.0, GridQuality::Auto),
            ThumbnailSize::Preview
        ));
    }

    #[test]
    fn explicit_quality_overrides_row_height() {
        assert!(matches!(
            bucket_for_row_height(2000.0, GridQuality::Thumbnail),
            ThumbnailSize::Thumbnail
        ));
        assert!(matches!(
            bucket_for_row_height(50.0, GridQuality::Fullsize),
            ThumbnailSize::Fullsize
        ));
    }

    #[test]
    fn fallback_chain_degrades_fullsize_to_preview() {
        assert_eq!(
            fallback_bucket(ThumbnailSize::Fullsize),
            Some(ThumbnailSize::Preview)
        );
        assert_eq!(fallback_bucket(ThumbnailSize::Preview), None);
        assert_eq!(fallback_bucket(ThumbnailSize::Thumbnail), None);
    }
}
