//! Sidebar navigation panel for the library window.
//!
//! Builds the split-pane sidebar containing album rows, fixed navigation
//! entries (All Assets, Explore, Local, Timeline), and the folder-scoped
//! album list. Handles selection changes to drive the main grid view source.

use gtk::prelude::*;
use libadwaita::prelude::*;

/// Contains references to the side navigation panel widgets and lists.
pub struct SidebarParts {
    pub root: gtk::Box,
    pub connection_row: libadwaita::ActionRow,
    pub server_row: libadwaita::ActionRow,
    pub fixed_list: gtk::ListBox,
    pub albums_list: gtk::ListBox,
}

/// Fixed sidebar destinations: title, subtitle, icon, and the key the sidebar handler matches.
const FIXED_ROWS: [(&str, &str, &str, &str); 8] = [
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
        "Favorites",
        "Photos and videos you've marked",
        "mimick-heart-filled-symbolic",
        "favorites",
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
    for (title, subtitle, icon, key) in FIXED_ROWS {
        fixed_list.append(&action_row(title, subtitle, icon, key));
    }

    let albums_header = gtk::Label::builder()
        .label("Albums")
        .xalign(0.0)
        .css_classes(vec!["heading".to_string(), "dim-label".to_string()])
        .margin_top(6)
        .build();

    let albums_list = selectable_list();

    // The fixed rows and albums scroll together so the sidebar fits short (720 px) screens;
    // only the connection footer stays pinned.
    let lists = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    lists.append(&fixed_list);
    lists.append(&albums_header);
    lists.append(&albums_list);
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
    }
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
