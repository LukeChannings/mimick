use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Datelike, NaiveDate};
use gtk::prelude::*;

use crate::api_client::{LibraryAlbum, MetadataSearchFilters, SharedLinkTarget};

use super::LibraryWindowUi;
use super::asset_tags::color_dot;
use super::dialogs::popover_menu;
use super::explore_view::spawn_person_thumbnail;
use super::state::{LibrarySource, Location};
use super::tree_page::{leaf_name, parent_path};

const AVATAR_SIZE: i32 = 64;

pub(super) struct SourceHeader {
    pub root: gtk::Box,
    pub extra: gtk::Box,
    pub status: gtk::Stack,
    leading: gtk::Box,
    title: gtk::Label,
    facts: gtk::Label,
    description: gtk::Label,
    actions: gtk::Box,
    status_error: gtk::Label,
    count: RefCell<Option<String>>,
    details: RefCell<Vec<String>>,
}

enum Leading {
    Icon(&'static str),
    Person { id: String, name: String },
    TagColor(Option<String>),
}

pub(super) fn build_source_header() -> SourceHeader {
    let leading = gtk::Box::builder().valign(gtk::Align::Center).build();
    let title = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["title-1"])
        .build();
    let facts = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["dim-label"])
        .visible(false)
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(&title);
    text.append(&facts);
    let actions = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    let top = gtk::Box::builder().spacing(12).build();
    top.append(&leading);
    top.append(&text);
    top.append(&actions);
    let description = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .visible(false)
        .build();
    let extra = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    root.append(&top);
    root.append(&description);
    root.append(&extra);

    let error_view = super::build_status_view(
        "dialog-warning-symbolic",
        "Library data unavailable",
        "Could not load library assets",
    );
    let status_error = error_view
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("status-view subtitle label");
    let status = gtk::Stack::builder()
        .vexpand(true)
        .vhomogeneous(false)
        .build();
    status.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("none"));
    status.add_named(&super::build_loading_view(), Some("loading"));
    status.add_named(
        &super::build_status_view(
            "image-x-generic-symbolic",
            "Nothing Here",
            "There are no photos or videos to show.",
        ),
        Some("empty"),
    );
    status.add_named(&error_view, Some("error"));

    SourceHeader {
        root,
        extra,
        status,
        leading,
        title,
        facts,
        description,
        actions,
        status_error,
        count: RefCell::default(),
        details: RefCell::default(),
    }
}

pub(super) fn set_status(ui: &LibraryWindowUi, status: &str, error: Option<&str>) {
    if let Some(error) = error {
        ui.header.status_error.set_label(error);
    }
    ui.header.status.set_visible_child_name(status);
    ui.header.status.set_visible(status != "none");
    ui.grid.canvas.set_visible(status == "none");
}

pub(super) fn refresh(ui: &Rc<LibraryWindowUi>) {
    if let Location::Source(source) = super::navigation::current(ui) {
        show(ui, &source);
    }
}

pub(super) fn set_loaded_count(ui: &LibraryWindowUi, count: usize) {
    *ui.header.count.borrow_mut() = Some(count_label(count as u64));
    render_facts(&ui.header);
}

pub(super) fn show(ui: &Rc<LibraryWindowUi>, source: &LibrarySource) {
    let header = &ui.header;
    *header.count.borrow_mut() = None;
    header.details.borrow_mut().clear();
    render_facts(header);
    set_description(header, None);
    clear_actions(header);
    let (leading, title) = basics(ui, source);
    set_leading(ui, leading);
    header.title.set_label(&title);
    header.title.set_tooltip_text(Some(&title));
    enrich(ui, source);
}

fn basics(ui: &LibraryWindowUi, source: &LibrarySource) -> (Leading, String) {
    let icon = |name: &'static str, title: &str| (Leading::Icon(name), title.to_string());
    match source {
        LibrarySource::AllAssets | LibrarySource::Timeline | LibrarySource::Unified => {
            if let Some(stats) = ui.ctx.library_state.lock().status.stats.clone() {
                *ui.header.details.borrow_mut() = vec![
                    count_noun(stats.images, "photo", "photos"),
                    count_noun(stats.videos, "video", "videos"),
                ];
            }
            icon("image-x-generic-symbolic", "Photos")
        }
        LibrarySource::LocalAll => icon("mimick-computer-symbolic", "On This Computer"),
        LibrarySource::Explore => icon("view-grid-symbolic", "Explore"),
        LibrarySource::Favorites => icon("mimick-heart-filled-symbolic", "Favorites"),
        LibrarySource::Archive => icon("mimick-archive-symbolic", "Archive"),
        LibrarySource::Trash => icon("user-trash-symbolic", "Trash"),
        LibrarySource::Album { name, .. }
        | LibrarySource::AlbumLocal { name, .. }
        | LibrarySource::AlbumUnified { name, .. } => icon("folder-pictures-symbolic", name),
        LibrarySource::Person { id, name } => (
            Leading::Person {
                id: id.clone(),
                name: name.clone(),
            },
            name.clone(),
        ),
        LibrarySource::Tag { name, .. } => (Leading::TagColor(None), leaf_name(name).to_string()),
        LibrarySource::Folder { path } => icon("folder-symbolic", leaf_name(path)),
        LibrarySource::Place { city } => icon("mark-location-symbolic", city),
        LibrarySource::Thing { query } => icon("system-search-symbolic", &capitalize(query)),
        LibrarySource::SmartSearch { query }
        | LibrarySource::MetadataSearch { query }
        | LibrarySource::OcrSearch { query }
        | LibrarySource::LocalSearch { query }
        | LibrarySource::UnifiedSearch { query } => icon("system-search-symbolic", query),
        LibrarySource::AdvancedSearch { .. } => icon("system-search-symbolic", "Search Results"),
    }
}

fn enrich(ui: &Rc<LibraryWindowUi>, source: &LibrarySource) {
    match source {
        LibrarySource::Favorites => count_matching(
            ui,
            MetadataSearchFilters {
                is_favorite: Some(true),
                ..MetadataSearchFilters::default()
            },
        ),
        LibrarySource::Archive => count_matching(
            ui,
            MetadataSearchFilters {
                visibility: Some("archive".to_string()),
                ..MetadataSearchFilters::default()
            },
        ),
        LibrarySource::Place { city } => count_matching(
            ui,
            MetadataSearchFilters {
                city: Some(city.clone()),
                ..MetadataSearchFilters::default()
            },
        ),
        LibrarySource::MetadataSearch { query } => count_matching(
            ui,
            MetadataSearchFilters {
                original_file_name: Some(query.clone()),
                ..MetadataSearchFilters::default()
            },
        ),
        LibrarySource::AdvancedSearch { filters } => count_matching(ui, (**filters).clone()),
        LibrarySource::Thing { .. } => set_details(&ui.header, vec!["Things".to_string()]),
        LibrarySource::SmartSearch { .. } => {
            set_details(&ui.header, vec!["Smart search".to_string()])
        }
        LibrarySource::Folder { path } => {
            set_details(&ui.header, vec![path.clone()]);
        }
        LibrarySource::Album { id, .. }
        | LibrarySource::AlbumLocal { id, .. }
        | LibrarySource::AlbumUnified { id, .. } => enrich_album(ui, id.clone()),
        LibrarySource::Person { id, .. } => enrich_person(ui, id.clone()),
        LibrarySource::Tag { id, name } => enrich_tag(ui, id.clone(), name.clone()),
        _ => {}
    }
}

fn generation(ui: &LibraryWindowUi) -> u64 {
    ui.ctx.library_state.lock().generation
}

fn spawn_for_current<F, Fut>(ui: &Rc<LibraryWindowUi>, task: F)
where
    F: FnOnce(Rc<LibraryWindowUi>) -> Fut + 'static,
    Fut: Future<Output = Option<Box<dyn FnOnce(&Rc<LibraryWindowUi>)>>> + 'static,
{
    let ui = ui.clone();
    let started = generation(&ui);
    glib::MainContext::default().spawn_local(async move {
        let apply = task(ui.clone()).await;
        if let Some(apply) = apply
            && generation(&ui) == started
        {
            apply(&ui);
        }
    });
}

fn count_matching(ui: &Rc<LibraryWindowUi>, filters: MetadataSearchFilters) {
    spawn_for_current(ui, move |ui| async move {
        let total = ui.ctx.api_client.search_statistics(&filters).await.ok()?;
        Some(Box::new(move |ui: &Rc<LibraryWindowUi>| {
            *ui.header.count.borrow_mut() = Some(count_label(total));
            render_facts(&ui.header);
        }) as Box<dyn FnOnce(&Rc<LibraryWindowUi>)>)
    });
}

fn enrich_album(ui: &Rc<LibraryWindowUi>, album_id: String) {
    spawn_for_current(ui, move |ui| async move {
        let album = ui.ctx.api_client.fetch_album_detail(&album_id).await.ok()?;
        let me = super::ensure_current_user_id(&ui.ctx)
            .await
            .unwrap_or_default();
        Some(
            Box::new(move |ui: &Rc<LibraryWindowUi>| show_album(ui, album, &me))
                as Box<dyn FnOnce(&Rc<LibraryWindowUi>)>,
        )
    });
}

fn show_album(ui: &Rc<LibraryWindowUi>, album: LibraryAlbum, me: &str) {
    let header = &ui.header;
    header.title.set_label(&album.album_name);
    *header.count.borrow_mut() = Some(count_label(album.asset_count.into()));
    let mut details = Vec::new();
    if let Some(range) = format_date_range(album.start_date.as_deref(), album.end_date.as_deref()) {
        details.push(range);
    }
    let owner_id = album.owner_id().to_string();
    if !owner_id.is_empty() && owner_id != me {
        let owner = album
            .album_users
            .iter()
            .find(|member| member.user.id == owner_id)
            .map(|member| member.user.name.clone())
            .unwrap_or_default();
        if !owner.is_empty() {
            details.push(format!("Shared by {owner}"));
        }
    } else {
        let others = album.album_users.len().saturating_sub(1);
        if others > 0 {
            details.push(match others {
                1 => "Shared with 1 person".to_string(),
                n => format!("Shared with {n} people"),
            });
        }
    }
    set_details(header, details);
    set_description(header, Some(&album.description));
    if owner_id.is_empty() || owner_id == me {
        let ui_share = ui.clone();
        let share_album = album.clone();
        add_action(header, "emblem-shared-symbolic", "Share album", move || {
            super::album_share_dialog::present(ui_share.clone(), share_album.clone())
        });
    }
    let (ui_link, album_id) = (ui.clone(), album.id.clone());
    add_action(
        header,
        "insert-link-symbolic",
        "Create share link",
        move || {
            super::shared_link_dialog::present(
                ui_link.clone(),
                super::shared_link_dialog::LinkDialogMode::Create(SharedLinkTarget::Album(
                    album_id.clone(),
                )),
                Rc::new(|| {}),
            )
        },
    );
}

fn enrich_person(ui: &Rc<LibraryWindowUi>, person_id: String) {
    spawn_for_current(ui, move |ui| async move {
        let person = ui.ctx.api_client.fetch_person(&person_id).await.ok()?;
        let count = ui.ctx.api_client.person_asset_count(&person_id).await.ok();
        Some(Box::new(move |ui: &Rc<LibraryWindowUi>| {
            let header = &ui.header;
            let name = if person.name.is_empty() {
                "Unnamed".to_string()
            } else {
                person.name.clone()
            };
            header.title.set_label(&name);
            *header.count.borrow_mut() = count.map(count_label);
            let today = chrono::Local::now().date_naive();
            let mut details = person_details(person.birth_date.as_deref(), today);
            if person.is_hidden {
                details.push("Hidden".to_string());
            }
            set_details(header, details);
            let menu = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .tooltip_text("More")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            popover_menu(&menu, super::people_view::person_menu_items(ui, &person));
            header.actions.append(&menu);
        }) as Box<dyn FnOnce(&Rc<LibraryWindowUi>)>)
    });
}

fn enrich_tag(ui: &Rc<LibraryWindowUi>, tag_id: String, value: String) {
    let mut details = Vec::new();
    if let Some(parent) = parent_path(&value) {
        details.push(format!("In {}", parent.replace('/', " / ")));
    }
    set_details(&ui.header, details);
    count_matching(
        ui,
        MetadataSearchFilters {
            tag_ids: Some(vec![tag_id.clone()]),
            ..MetadataSearchFilters::default()
        },
    );
    spawn_for_current(ui, move |ui| async move {
        let tags = ui.ctx.api_client.fetch_tags().await.ok()?;
        let tag = tags.into_iter().find(|tag| tag.id == tag_id)?;
        Some(Box::new(move |ui: &Rc<LibraryWindowUi>| {
            let header = &ui.header;
            header.title.set_label(&tag.name);
            set_leading(ui, Leading::TagColor(tag.color.clone()));
            let menu = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .tooltip_text("More")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            popover_menu(&menu, super::tags_view::tag_menu_items(ui, &tag));
            header.actions.append(&menu);
        }) as Box<dyn FnOnce(&Rc<LibraryWindowUi>)>)
    });
}

fn set_leading(ui: &LibraryWindowUi, leading: Leading) {
    let slot = &ui.header.leading;
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
    let widget: gtk::Widget = match leading {
        Leading::Icon(name) => gtk::Image::builder()
            .icon_name(name)
            .pixel_size(32)
            .css_classes(["dim-label"])
            .build()
            .upcast(),
        Leading::Person { id, name } => {
            let avatar = libadwaita::Avatar::new(AVATAR_SIZE, Some(&name), true);
            let avatar_for_image = avatar.clone();
            spawn_person_thumbnail(ui.ctx.clone(), id, move |texture| {
                avatar_for_image.set_custom_image(Some(texture))
            });
            avatar.upcast()
        }
        Leading::TagColor(color) => color_dot(color.as_deref(), 20).upcast(),
    };
    slot.append(&widget);
}

fn clear_actions(header: &SourceHeader) {
    while let Some(child) = header.actions.first_child() {
        header.actions.remove(&child);
    }
}

fn add_action(header: &SourceHeader, icon: &str, tooltip: &str, on_click: impl Fn() + 'static) {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.connect_clicked(move |_| on_click());
    header.actions.append(&button);
}

fn set_details(header: &SourceHeader, details: Vec<String>) {
    *header.details.borrow_mut() = details;
    render_facts(header);
}

fn set_description(header: &SourceHeader, description: Option<&str>) {
    let text = description.map(str::trim).unwrap_or_default();
    header.description.set_label(text);
    header.description.set_visible(!text.is_empty());
}

fn render_facts(header: &SourceHeader) {
    let parts: Vec<String> = header
        .count
        .borrow()
        .iter()
        .cloned()
        .chain(header.details.borrow().iter().cloned())
        .filter(|part| !part.is_empty())
        .collect();
    let text = parts.join(" · ");
    header.facts.set_label(&text);
    header.facts.set_visible(!text.is_empty());
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn count_noun(n: u64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        group_thousands(n),
        if n == 1 { singular } else { plural }
    )
}

fn count_label(n: u64) -> String {
    count_noun(n, "item", "items")
}

fn parse_day(value: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.date_naive())
        .ok()
        .or_else(|| NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d").ok())
}

fn age_on(birth: NaiveDate, today: NaiveDate) -> i32 {
    let mut age = today.year() - birth.year();
    if (today.month(), today.day()) < (birth.month(), birth.day()) {
        age -= 1;
    }
    age
}

fn person_details(birth_date: Option<&str>, today: NaiveDate) -> Vec<String> {
    let Some(birth) = birth_date.and_then(parse_day) else {
        return Vec::new();
    };
    vec![
        format!("Age {}", age_on(birth, today)),
        format!("Born {}", birth.format("%-d %b %Y")),
    ]
}

fn format_date_range(start: Option<&str>, end: Option<&str>) -> Option<String> {
    let start = start.and_then(parse_day);
    let end = end.and_then(parse_day);
    let (start, end) = match (start, end) {
        (Some(start), Some(end)) => (start.min(end), start.max(end)),
        (Some(day), None) | (None, Some(day)) => (day, day),
        (None, None) => return None,
    };
    let full = |day: NaiveDate| day.format("%-d %b %Y").to_string();
    Some(if start == end {
        full(start)
    } else if start.year() == end.year() && start.month() == end.month() {
        format!("{}–{}", start.format("%-d"), full(end))
    } else if start.year() == end.year() {
        format!("{} – {}", start.format("%-d %b"), full(end))
    } else {
        format!("{} – {}", full(start), full(end))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn age_counts_whole_years() {
        assert_eq!(age_on(day(1990, 5, 4), day(2026, 5, 3)), 35);
        assert_eq!(age_on(day(1990, 5, 4), day(2026, 5, 4)), 36);
    }

    #[test]
    fn person_details_show_age_and_birthday() {
        assert_eq!(
            person_details(Some("1990-05-04"), day(2026, 10, 12)),
            ["Age 36", "Born 4 May 1990"]
        );
        assert!(person_details(None, day(2026, 10, 12)).is_empty());
    }

    #[test]
    fn date_ranges_collapse_shared_parts() {
        let range = |a: &str, b: &str| format_date_range(Some(a), Some(b)).unwrap();
        assert_eq!(
            range("2024-05-04T10:00:00.000Z", "2024-05-04T18:00:00.000Z"),
            "4 May 2024"
        );
        assert_eq!(
            range("2024-05-04T10:00:00.000Z", "2024-05-09T10:00:00.000Z"),
            "4–9 May 2024"
        );
        assert_eq!(
            range("2024-05-04T10:00:00.000Z", "2024-06-09T10:00:00.000Z"),
            "4 May – 9 Jun 2024"
        );
        assert_eq!(
            range("2023-12-30T10:00:00.000Z", "2024-01-02T10:00:00.000Z"),
            "30 Dec 2023 – 2 Jan 2024"
        );
        assert_eq!(format_date_range(None, None), None);
    }

    #[test]
    fn counts_group_thousands_and_pluralize() {
        assert_eq!(count_label(1), "1 item");
        assert_eq!(count_label(1234567), "1,234,567 items");
        assert_eq!(count_noun(0, "photo", "photos"), "0 photos");
    }

    #[test]
    fn capitalize_first_letter_only() {
        assert_eq!(capitalize("beach"), "Beach");
        assert_eq!(capitalize(""), "");
    }
}
