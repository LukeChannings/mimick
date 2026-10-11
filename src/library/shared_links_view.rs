//! "Shared Links" page: every public link the user has made, with copy, edit, and delete.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::SharedLink;

use super::LibraryWindowUi;
use super::context_menu::show_alert_dialog;
use super::shared_link_dialog::{self, LinkDialogMode, copy_link_url, format_expiry};

/// Widgets for the Shared Links content page.
pub(super) struct SharedLinksView {
    pub root: gtk::ScrolledWindow,
    list: libadwaita::PreferencesGroup,
    empty: libadwaita::StatusPage,
    rows: Rc<RefCell<Vec<gtk::Widget>>>,
}

pub(super) fn build_shared_links_view() -> SharedLinksView {
    let list = libadwaita::PreferencesGroup::builder()
        .title("Shared Links")
        .description("Anyone with a link can view what it shares.")
        .build();
    let empty = libadwaita::StatusPage::builder()
        .icon_name("emblem-shared-symbolic")
        .title("No Shared Links")
        .description(
            "Create one from an album's or a photo's right-click menu, or from a selection.",
        )
        .visible(false)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&list);
    content.append(&empty);
    let clamp = libadwaita::Clamp::builder()
        .maximum_size(820)
        .child(&content)
        .build();
    let root = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    SharedLinksView {
        root,
        list,
        empty,
        rows: Rc::default(),
    }
}

/// Show the Shared Links page and (re)load its links.
pub(super) fn show(ui: &Rc<LibraryWindowUi>) {
    ui.content_stack.set_visible_child_name("shared-links");
    reload(ui.clone());
}

fn reload(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_shared_links().await {
            Ok(links) => render(&ui, &links),
            Err(err) => show_alert_dialog(
                &ui,
                "Could Not Load Shared Links",
                &format!("{err}\n\nYour API key needs the 'sharedLink.read' permission."),
            ),
        }
    });
}

fn render(ui: &Rc<LibraryWindowUi>, links: &[SharedLink]) {
    let view = &ui.shared_links;
    for row in view.rows.borrow_mut().drain(..) {
        view.list.remove(&row);
    }
    for link in links {
        let row = link_row(ui, link);
        view.list.add(&row);
        view.rows.borrow_mut().push(row.upcast());
    }
    view.list.set_visible(!links.is_empty());
    view.empty.set_visible(links.is_empty());
}

fn link_row(ui: &Rc<LibraryWindowUi>, link: &SharedLink) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title(glib::markup_escape_text(&link_title(link)))
        .subtitle(glib::markup_escape_text(&link_subtitle(link)))
        .subtitle_lines(2)
        .build();
    let copy = icon_button("edit-copy-symbolic", "Copy link");
    let edit = icon_button("document-edit-symbolic", "Edit link");
    let delete = icon_button("user-trash-symbolic", "Delete link");
    let (ui_copy, link_copy) = (ui.clone(), link.clone());
    copy.connect_clicked(move |_| {
        let (ui, link) = (ui_copy.clone(), link_copy.clone());
        glib::MainContext::default().spawn_local(async move {
            copy_link_url(&ui, &link, "Link Copied").await;
        });
    });
    let (ui_edit, link_edit) = (ui.clone(), link.clone());
    edit.connect_clicked(move |_| {
        let ui_after = ui_edit.clone();
        shared_link_dialog::present(
            ui_edit.clone(),
            LinkDialogMode::Edit(Box::new(link_edit.clone())),
            Rc::new(move || reload(ui_after.clone())),
        );
    });
    let (ui_delete, link_id) = (ui.clone(), link.id.clone());
    delete.connect_clicked(move |_| confirm_delete(ui_delete.clone(), link_id.clone()));
    row.add_suffix(&copy);
    row.add_suffix(&edit);
    row.add_suffix(&delete);
    row
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

fn confirm_delete(ui: Rc<LibraryWindowUi>, link_id: String) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Delete this link?")
        .body("Anyone using it will lose access. This can't be undone.")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    let ui_for_choice = ui.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let (ui, link_id) = (ui_for_choice.clone(), link_id.clone());
        glib::MainContext::default().spawn_local(async move {
            match ui.ctx.api_client.delete_shared_link(&link_id).await {
                Ok(()) => reload(ui),
                Err(err) => show_alert_dialog(&ui, "Could Not Delete Link", &err),
            }
        });
    });
    dialog.present(Some(&ui.window));
}

/// Album name for album links, otherwise the number of shared items.
fn link_title(link: &SharedLink) -> String {
    match &link.album {
        Some(album) => album.album_name.clone(),
        None => match link.assets.len() {
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        },
    }
}

/// Description, expiry, and any restrictions, joined with " · ".
fn link_subtitle(link: &SharedLink) -> String {
    let mut parts = Vec::new();
    if let Some(description) = link.description.as_deref().filter(|d| !d.is_empty()) {
        parts.push(description.to_string());
    }
    parts.push(match link.expires_at.as_deref() {
        Some(at) => format!("Expires {}", format_expiry(at)),
        None => "Never expires".to_string(),
    });
    if link.password.is_some() {
        parts.push("Password".to_string());
    }
    if !link.allow_download {
        parts.push("No downloads".to_string());
    }
    if link.allow_upload {
        parts.push("Uploads allowed".to_string());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(json: serde_json::Value) -> SharedLink {
        let mut base = serde_json::json!({
            "id": "l1",
            "key": "k",
            "type": "INDIVIDUAL",
            "createdAt": "2026-10-01T00:00:00.000Z",
            "allowDownload": true,
            "allowUpload": false,
            "showMetadata": true,
            "assets": [{ "id": "a" }, { "id": "b" }]
        });
        for (key, value) in json.as_object().unwrap() {
            base[key] = value.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn title_uses_album_name_or_item_count() {
        assert_eq!(link_title(&link(serde_json::json!({}))), "2 items");
        let album = link(serde_json::json!({
            "type": "ALBUM",
            "album": { "id": "a1", "albumName": "Trips" },
            "assets": []
        }));
        assert_eq!(link_title(&album), "Trips");
        let single = link(serde_json::json!({ "assets": [{ "id": "a" }] }));
        assert_eq!(link_title(&single), "1 item");
    }

    #[test]
    fn subtitle_lists_description_expiry_and_restrictions() {
        assert_eq!(link_subtitle(&link(serde_json::json!({}))), "Never expires");
        let restricted = link(serde_json::json!({
            "description": "Beach",
            "password": "x",
            "allowDownload": false,
            "allowUpload": true
        }));
        assert_eq!(
            link_subtitle(&restricted),
            "Beach · Never expires · Password · No downloads · Uploads allowed"
        );
        let expiring = link(serde_json::json!({ "expiresAt": "2026-10-09T12:00:00Z" }));
        assert!(link_subtitle(&expiring).starts_with("Expires "));
    }
}
