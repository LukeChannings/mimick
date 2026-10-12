//! Sidebar navigation panel for the library window.
//!
//! Builds the split-pane sidebar containing album rows, fixed navigation
//! entries (All Assets, Explore, Local, Timeline), and the folder-scoped
//! album list. Handles selection changes to drive the main grid view source.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::LibraryAlbum;

const RECENT_ALBUM_COUNT: usize = 3;
const RECENT_ALBUM_PREFIX: &str = "recent-album:";

/// Contains references to the side navigation panel widgets and lists.
pub struct SidebarParts {
    pub root: gtk::Box,
    pub connection_row: libadwaita::ActionRow,
    pub server_row: libadwaita::ActionRow,
    pub fixed_list: gtk::ListBox,
    pub albums_list: gtk::ListBox,
    recent_expander: gtk::ToggleButton,
    recent_rows: Rc<RefCell<Vec<gtk::ListBoxRow>>>,
}

/// Fixed sidebar destinations: title, subtitle, icon, and the key the sidebar handler matches.
const FIXED_ROWS: [(&str, &str, &str, &str); 13] = [
    (
        "Photos",
        "Timeline of every photo and video",
        "image-x-generic-symbolic",
        "photos",
    ),
    (
        "Search",
        "Find photos, videos, and text",
        "system-search-symbolic",
        "search",
    ),
    (
        "Explore",
        "People, places, and things",
        "view-grid-symbolic",
        "explore",
    ),
    (
        "People",
        "Faces Immich has recognized",
        "avatar-default-symbolic",
        "people",
    ),
    (
        "Albums",
        "Recent, owned, and shared",
        "folder-pictures-symbolic",
        "albums",
    ),
    (
        "Shared Albums",
        "Shared with you and by you",
        "folder-publicshare-symbolic",
        "shared-albums",
    ),
    (
        "Shared Links",
        "Public links you've created",
        "emblem-shared-symbolic",
        "shared-links",
    ),
    (
        "Sharing",
        "Partners who share libraries",
        "system-users-symbolic",
        "sharing",
    ),
    (
        "Favorites",
        "Photos and videos you've marked",
        "mimick-heart-filled-symbolic",
        "favorites",
    ),
    (
        "Tags",
        "Browse and organize tags",
        "mimick-tag-symbolic",
        "tags",
    ),
    (
        "Folders",
        "Browse by folder on the server",
        "folder-symbolic",
        "folders",
    ),
    (
        "Archive",
        "Hidden from Photos",
        "mimick-archive-symbolic",
        "archive",
    ),
    (
        "Trash",
        "Restore or permanently delete",
        "user-trash-symbolic",
        "trash",
    ),
];

/// Connection and server rows at the bottom of the sidebar, in their own list.
fn build_footer() -> (gtk::ListBox, libadwaita::ActionRow, libadwaita::ActionRow) {
    let connection_row = footer_row("Connection", "Offline");
    let server_row = footer_row("Server", "Statistics unavailable");
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    list.append(&connection_row);
    list.append(&server_row);
    (list, connection_row, server_row)
}

/// Single-line activatable row for the connection/server footer.
fn footer_row(title: &str, subtitle: &str) -> libadwaita::ActionRow {
    libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .title_lines(1)
        .subtitle_lines(1)
        .activatable(true)
        .build()
}

/// Single-selection boxed list for the fixed entries and the album rows.
fn selectable_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build()
}

/// Construct the hierarchical side navigation sidebar panel.
pub fn build_sidebar() -> SidebarParts {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .width_request(200)
        .build();

    let fixed_list = selectable_list();
    let (connection_list, connection_row, server_row) = build_footer();
    let recent_expander = gtk::ToggleButton::builder()
        .icon_name("pan-end-symbolic")
        .tooltip_text("Show recent albums")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .visible(false)
        .build();
    let recent_rows: Rc<RefCell<Vec<gtk::ListBoxRow>>> = Rc::default();
    for (title, subtitle, icon, key) in FIXED_ROWS {
        let row = action_row(title, subtitle, icon, key);
        if key == "albums" {
            row.add_suffix(&recent_expander);
        }
        fixed_list.append(&row);
    }
    let rows = recent_rows.clone();
    recent_expander.connect_toggled(move |button| {
        let expanded = button.is_active();
        button.set_icon_name(if expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        button.set_tooltip_text(Some(if expanded {
            "Hide recent albums"
        } else {
            "Show recent albums"
        }));
        for row in rows.borrow().iter() {
            row.set_visible(expanded);
        }
    });

    let albums_header = gtk::Label::builder()
        .label("All Albums")
        .xalign(0.0)
        .css_classes(vec!["heading".to_string(), "dim-label".to_string()])
        .build();

    let albums_list = selectable_list();
    let albums_expander = gtk::Expander::builder()
        .label_widget(&albums_header)
        .expanded(true)
        .margin_top(6)
        .child(&albums_list)
        .build();
    albums_list.set_margin_top(12);

    // The fixed rows and albums scroll together so the sidebar fits short (720 px) screens;
    // only the connection footer stays pinned.
    let lists = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    lists.append(&fixed_list);
    lists.append(&albums_expander);
    let lists_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&lists)
        .build();

    root.append(&lists_scroll);
    root.append(&connection_list);

    SidebarParts {
        root,
        connection_row,
        server_row,
        fixed_list,
        albums_list,
        recent_expander,
        recent_rows,
    }
}

pub fn set_recent_albums(parts: &SidebarParts, albums: &[LibraryAlbum]) {
    let mut rows = parts.recent_rows.borrow_mut();
    for row in rows.drain(..) {
        parts.fixed_list.remove(&row);
    }
    let Some(position) = fixed_row_index("albums") else {
        return;
    };
    let expanded = parts.recent_expander.is_active();
    for (offset, album) in recent_albums(albums).into_iter().enumerate() {
        let row = recent_album_row(album);
        row.set_visible(expanded);
        parts
            .fixed_list
            .insert(&row, (position + 1 + offset) as i32);
        rows.push(row);
    }
    parts.recent_expander.set_visible(!rows.is_empty());
}

pub fn recent_album_key(row: &gtk::ListBoxRow) -> Option<(String, String)> {
    let name = row.widget_name();
    let (id, album_name) = name.strip_prefix(RECENT_ALBUM_PREFIX)?.split_once(':')?;
    Some((id.to_string(), album_name.to_string()))
}

fn fixed_row_index(key: &str) -> Option<usize> {
    FIXED_ROWS.iter().position(|(_, _, _, k)| *k == key)
}

fn recent_albums(albums: &[LibraryAlbum]) -> Vec<&LibraryAlbum> {
    let mut recent: Vec<&LibraryAlbum> = albums.iter().collect();
    recent.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    recent.truncate(RECENT_ALBUM_COUNT);
    recent
}

fn recent_album_row(album: &LibraryAlbum) -> gtk::ListBoxRow {
    let row = libadwaita::ActionRow::builder()
        .title(&album.album_name)
        .use_markup(false)
        .title_lines(1)
        .activatable(true)
        .name(format!(
            "{RECENT_ALBUM_PREFIX}{}:{}",
            album.id, album.album_name
        ))
        .build();
    let icon = gtk::Image::builder()
        .icon_name("folder-pictures-symbolic")
        .margin_start(16)
        .css_classes(["dim-label"])
        .build();
    row.add_prefix(&icon);
    row.upcast()
}

/// Construct a navigation list row decorated with an icon and titles.
fn action_row(title: &str, subtitle: &str, icon_name: &str, key: &str) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .title_lines(1)
        .subtitle_lines(1)
        .tooltip_text(key)
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name(icon_name);
    row.add_prefix(&icon);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str, updated_at: &str) -> LibraryAlbum {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "albumName": format!("Album {id}"),
            "assetCount": 1,
            "albumThumbnailAssetId": null,
            "createdAt": "2024-01-01T00:00:00.000Z",
            "updatedAt": updated_at
        }))
        .unwrap()
    }

    #[test]
    fn recent_albums_are_the_latest_updated_first() {
        let albums = vec![
            album("a", "2024-01-01T00:00:00.000Z"),
            album("b", "2024-03-01T00:00:00.000Z"),
            album("c", "2024-02-01T00:00:00.000Z"),
            album("d", "2024-04-01T00:00:00.000Z"),
        ];
        let ids: Vec<&str> = recent_albums(&albums)
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, ["d", "b", "c"]);
    }

    #[test]
    fn albums_row_has_a_fixed_index() {
        assert_eq!(fixed_row_index("albums"), Some(4));
        assert_eq!(fixed_row_index("missing"), None);
    }
}
