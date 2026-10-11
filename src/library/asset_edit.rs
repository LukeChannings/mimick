//! "Edit Info" dialog: change an asset's description, capture date, and favorite flag.
//!
//! Only fields the user actually changed are sent to Immich, so an untouched
//! date never gets rewritten with a lossy local-time round trip.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{AssetDetails, AssetUpdate};

use super::LibraryWindowUi;
use super::context_menu::show_alert_dialog;
use super::favorites::{apply_saved_favorite, drop_from_favorites_view, heart_icon};

/// Format shown in and accepted by the "Date taken" field (local time).
const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Field values as shown in the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditValues {
    description: String,
    date_text: String,
    favorite: bool,
}

impl EditValues {
    fn from_details(details: &AssetDetails) -> Self {
        let exif = details.exif_info.as_ref();
        Self {
            description: exif.and_then(|e| e.description.clone()).unwrap_or_default(),
            date_text: exif
                .and_then(|e| e.date_time_original.as_deref())
                .and_then(format_local_datetime)
                .unwrap_or_default(),
            favorite: details.is_favorite,
        }
    }
}

/// Immich stores capture times in UTC; show them in the user's local timezone.
fn format_local_datetime(iso: &str) -> Option<String> {
    let local = DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.with_timezone(&Local))
        .or_else(|_| {
            iso.parse::<DateTime<Utc>>()
                .map(|dt| dt.with_timezone(&Local))
        })
        .ok()?;
    Some(local.format(DATE_FORMAT).to_string())
}

/// Read `text` as local time and return RFC3339 with its UTC offset, which Immich accepts.
/// `None` for malformed input or a time skipped by a DST change.
fn parse_local_datetime(text: &str) -> Option<String> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), DATE_FORMAT).ok()?;
    let local = Local.from_local_datetime(&naive).earliest()?;
    Some(local.to_rfc3339())
}

/// The update to send, holding only changed fields; `None` if the edited date is invalid.
fn update_from(original: &EditValues, edited: &EditValues) -> Option<AssetUpdate> {
    let date_time_original = if edited.date_text == original.date_text {
        None
    } else {
        Some(parse_local_datetime(&edited.date_text)?)
    };
    Some(AssetUpdate {
        description: (edited.description != original.description)
            .then(|| edited.description.clone()),
        date_time_original,
        is_favorite: (edited.favorite != original.favorite).then_some(edited.favorite),
    })
}

/// Fetch the asset's current info and open the edit dialog.
/// `on_saved` receives whether the capture date changed (the timeline order may move).
pub(super) fn show_edit_dialog(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    filename: String,
    on_saved: Rc<dyn Fn(bool)>,
) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_asset_details(&asset_id).await {
            Ok(details) => {
                let original = EditValues::from_details(&details);
                present_edit_dialog(ui, asset_id, &filename, original, on_saved);
            }
            Err(err) => show_alert_dialog(&ui, "Could Not Load Info", &err),
        }
    });
}

/// The editable rows inside the dialog.
struct EditForm {
    group: libadwaita::PreferencesGroup,
    description_row: libadwaita::EntryRow,
    date: DateField,
    favorite_button: gtk::ToggleButton,
}

/// "Date taken" row showing the chosen local date, with a calendar + time picker.
struct DateField {
    row: libadwaita::ActionRow,
    /// Local date in `DATE_FORMAT`, or empty when the asset has no capture date.
    text: Rc<RefCell<String>>,
}

impl DateField {
    fn new(original: &str) -> Self {
        let text = Rc::new(RefCell::new(original.to_string()));
        let row = libadwaita::ActionRow::builder()
            .title("Date taken")
            .subtitle(display_date(original))
            .build();
        let button = gtk::MenuButton::builder()
            .icon_name("x-office-calendar-symbolic")
            .tooltip_text("Pick date and time")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.set_popover(Some(&date_popover(&row, &text)));
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        Self { row, text }
    }
}

/// Calendar plus hour/minute/second spinners for picking a local date and time.
struct DatePicker {
    calendar: gtk::Calendar,
    hour: gtk::SpinButton,
    minute: gtk::SpinButton,
    second: gtk::SpinButton,
}

impl DatePicker {
    fn new(parts: DateParts) -> Self {
        let calendar = gtk::Calendar::new();
        if let Ok(day) =
            glib::DateTime::from_local(parts.0, parts.1 as i32, parts.2 as i32, 0, 0, 0.0)
        {
            calendar.set_date(&day);
        }
        let spin = |max: f64, value: u32| {
            let button = gtk::SpinButton::with_range(0.0, max, 1.0);
            button.set_value(f64::from(value));
            button.set_wrap(true);
            button
        };
        Self {
            calendar,
            hour: spin(23.0, parts.3),
            minute: spin(59.0, parts.4),
            second: spin(59.0, parts.5),
        }
    }

    /// "HH : MM : SS" spinner row.
    fn time_row(&self) -> gtk::Box {
        let time = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Center)
            .build();
        time.append(&self.hour);
        time.append(&gtk::Label::new(Some(":")));
        time.append(&self.minute);
        time.append(&gtk::Label::new(Some(":")));
        time.append(&self.second);
        time
    }

    fn picked_text(&self) -> Option<String> {
        let day = self.calendar.date();
        compose_date_text(
            day.year(),
            day.month() as u32,
            day.day_of_month() as u32,
            self.hour.value() as u32,
            self.minute.value() as u32,
            self.second.value() as u32,
        )
    }
}

/// Popover with the date picker; "Set Date" writes the result back to the row.
fn date_popover(row: &libadwaita::ActionRow, text: &Rc<RefCell<String>>) -> gtk::Popover {
    let picker = DatePicker::new(date_parts(&text.borrow()).unwrap_or_else(now_parts));
    let apply = gtk::Button::builder()
        .label("Set Date")
        .css_classes(["suggested-action"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    content.append(&picker.calendar);
    content.append(&picker.time_row());
    content.append(&apply);
    let popover = gtk::Popover::builder().child(&content).build();
    let (row, text, popover_ref) = (row.clone(), text.clone(), popover.clone());
    apply.connect_clicked(move |_| {
        if let Some(picked) = picker.picked_text() {
            row.set_subtitle(&picked);
            *text.borrow_mut() = picked;
        }
        popover_ref.popdown();
    });
    popover
}

type DateParts = (i32, u32, u32, u32, u32, u32);

/// `(year, month, day, hour, minute, second)` from a `DATE_FORMAT` string.
fn date_parts(text: &str) -> Option<DateParts> {
    use chrono::{Datelike, Timelike};
    let dt = NaiveDateTime::parse_from_str(text.trim(), DATE_FORMAT).ok()?;
    Some((
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    ))
}

/// Picker start for assets without a capture date: now, local time.
fn now_parts() -> DateParts {
    let now = Local::now().naive_local().format(DATE_FORMAT).to_string();
    date_parts(&now).unwrap_or((2000, 1, 1, 12, 0, 0))
}

/// `DATE_FORMAT` text for picked values, or `None` for an impossible date.
fn compose_date_text(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> Option<String> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    Some(date.and_hms_opt(h, m, s)?.format(DATE_FORMAT).to_string())
}

fn display_date(text: &str) -> String {
    if text.trim().is_empty() {
        "Not set".to_string()
    } else {
        text.to_string()
    }
}

/// "Favorite" row with a heart toggle (filled when on); clicking the row toggles it too.
fn favorite_row(favorite: bool) -> (libadwaita::ActionRow, gtk::ToggleButton) {
    let button = gtk::ToggleButton::builder()
        .icon_name(heart_icon(favorite))
        .active(favorite)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    button.connect_toggled(|button| button.set_icon_name(heart_icon(button.is_active())));
    let row = libadwaita::ActionRow::builder()
        .title("Favorite")
        .activatable_widget(&button)
        .build();
    row.add_suffix(&button);
    (row, button)
}

impl EditForm {
    fn new(original: &EditValues, filename: &str) -> Self {
        let description_row = libadwaita::EntryRow::builder()
            .title("Description")
            .text(&original.description)
            .build();
        let date = DateField::new(&original.date_text);
        let (favorite_row, favorite_button) = favorite_row(original.favorite);
        let group = libadwaita::PreferencesGroup::builder()
            .description(filename)
            .build();
        group.add(&description_row);
        group.add(&date.row);
        group.add(&favorite_row);
        Self {
            group,
            description_row,
            date,
            favorite_button,
        }
    }

    fn values(&self) -> EditValues {
        EditValues {
            description: self.description_row.text().to_string(),
            date_text: self.date.text.borrow().clone(),
            favorite: self.favorite_button.is_active(),
        }
    }
}

/// Dialog with Cancel/Save in the header bar around `form`; returns the dialog and both buttons.
fn dialog_shell(form: &EditForm) -> (libadwaita::Dialog, gtk::Button, gtk::Button) {
    let cancel_btn = gtk::Button::builder().label("Cancel").build();
    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    let header = libadwaita::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&form.group);
    let toolbar = libadwaita::ToolbarView::builder().content(&content).build();
    toolbar.add_top_bar(&header);
    let dialog = libadwaita::Dialog::builder()
        .title("Edit Info")
        .content_width(440)
        .child(&toolbar)
        .build();
    (dialog, cancel_btn, save_btn)
}

fn present_edit_dialog(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    filename: &str,
    original: EditValues,
    on_saved: Rc<dyn Fn(bool)>,
) {
    let form = EditForm::new(&original, filename);
    let (dialog, cancel_btn, save_btn) = dialog_shell(&form);

    let dialog_for_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });

    let dialog_for_save = dialog.clone();
    let ui_for_save = ui.clone();
    save_btn.connect_clicked(move |btn| {
        let Some(update) = update_from(&original, &form.values()) else {
            return;
        };
        if update == AssetUpdate::default() {
            dialog_for_save.close();
            return;
        }
        btn.set_sensitive(false);
        save_update(
            ui_for_save.clone(),
            asset_id.clone(),
            update,
            dialog_for_save.clone(),
            btn.clone(),
            on_saved.clone(),
        );
    });

    dialog.present(Some(&ui.window));
}

fn save_update(
    ui: Rc<LibraryWindowUi>,
    asset_id: String,
    update: AssetUpdate,
    dialog: libadwaita::Dialog,
    save_btn: gtk::Button,
    on_saved: Rc<dyn Fn(bool)>,
) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.update_asset(&asset_id, &update).await {
            Ok(()) => {
                dialog.close();
                if let Some(favorite) = update.is_favorite {
                    apply_saved_favorite(&ui, &asset_id, favorite);
                    if !favorite {
                        drop_from_favorites_view(&ui, &asset_id);
                    }
                }
                on_saved(update.date_time_original.is_some());
            }
            Err(err) => {
                // Re-enable Save so the user can retry after fixing permissions or connectivity.
                save_btn.set_sensitive(true);
                show_alert_dialog(
                    &ui,
                    "Could Not Save Info",
                    &format!("Immich rejected the update: {err}"),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(description: &str, date_text: &str, favorite: bool) -> EditValues {
        EditValues {
            description: description.into(),
            date_text: date_text.into(),
            favorite,
        }
    }

    #[test]
    fn edit_values_prefill_from_asset_details() {
        let details: AssetDetails = serde_json::from_value(serde_json::json!({
            "isFavorite": true,
            "exifInfo": {
                "description": "Beach day",
                "dateTimeOriginal": "2024-01-15T14:25:15.000Z"
            }
        }))
        .unwrap();
        let values = EditValues::from_details(&details);
        assert_eq!(values.description, "Beach day");
        assert!(values.favorite);
        assert_eq!(
            Some(values.date_text.clone()),
            format_local_datetime("2024-01-15T14:25:15.000Z")
        );
    }

    #[test]
    fn edit_values_are_empty_without_exif() {
        let details: AssetDetails = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(EditValues::from_details(&details), values("", "", false));
    }

    #[test]
    fn local_datetime_round_trips() {
        let rfc3339 = parse_local_datetime("2024-01-15 19:55:15").expect("valid date");
        assert_eq!(
            format_local_datetime(&rfc3339).as_deref(),
            Some("2024-01-15 19:55:15")
        );
    }

    #[test]
    fn date_parts_and_compose_round_trip() {
        let parts = date_parts("2024-01-15 19:55:15").unwrap();
        assert_eq!(parts, (2024, 1, 15, 19, 55, 15));
        assert_eq!(
            compose_date_text(2024, 1, 15, 19, 55, 15).as_deref(),
            Some("2024-01-15 19:55:15")
        );
        assert_eq!(compose_date_text(2024, 2, 30, 0, 0, 0), None);
        assert_eq!(compose_date_text(2024, 1, 1, 24, 0, 0), None);
        assert!(date_parts("").is_none());
    }

    #[test]
    fn display_date_marks_missing_dates() {
        assert_eq!(display_date(""), "Not set");
        assert_eq!(display_date("2024-01-15 19:55:15"), "2024-01-15 19:55:15");
        let (year, month, ..) = now_parts();
        assert!(year >= 2024 && (1..=12).contains(&month));
    }

    #[test]
    fn parse_rejects_malformed_dates() {
        assert_eq!(parse_local_datetime(""), None);
        assert_eq!(parse_local_datetime("yesterday"), None);
        assert_eq!(parse_local_datetime("2024-13-01 00:00:00"), None);
        assert_eq!(parse_local_datetime("2024-01-15"), None);
    }

    #[test]
    fn format_accepts_immich_utc_timestamps() {
        assert!(format_local_datetime("2024-01-15T14:25:15.000Z").is_some());
        assert_eq!(format_local_datetime("not a date"), None);
    }

    #[test]
    fn update_holds_only_changed_fields() {
        let original = values("Old", "2024-01-15 19:55:15", false);
        let edited = values("New", "2024-01-15 19:55:15", true);
        let update = update_from(&original, &edited).expect("valid edit");
        assert_eq!(update.description.as_deref(), Some("New"));
        assert_eq!(update.date_time_original, None);
        assert_eq!(update.is_favorite, Some(true));
    }

    #[test]
    fn update_is_empty_when_nothing_changed() {
        let original = values("Same", "", true);
        assert_eq!(
            update_from(&original, &original.clone()),
            Some(AssetUpdate::default())
        );
    }

    #[test]
    fn update_rejects_an_invalid_new_date() {
        let original = values("", "2024-01-15 19:55:15", false);
        let edited = values("", "2024-01-15", false);
        assert_eq!(update_from(&original, &edited), None);
    }

    #[test]
    fn update_sends_a_changed_date_with_its_offset() {
        let original = values("", "2024-01-15 19:55:15", false);
        let edited = values("", "2024-01-16 08:00:00", false);
        let date = update_from(&original, &edited)
            .and_then(|u| u.date_time_original)
            .expect("date update");
        assert!(date.starts_with("2024-01-16T08:00:00"));
    }
}
