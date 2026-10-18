use adw::prelude::*;
use gtk::{Button, Entry, PasswordEntry, Switch};
use libadwaita as adw;

pub struct ConnectivityWidgets {
    pub internal_switch: Switch,
    pub external_switch: Switch,
    pub internal_entry: Entry,
    pub external_entry: Entry,
    pub api_key_entry: PasswordEntry,
    pub test_btn: Button,
    pub save_btn: Button,
}

pub fn build_connectivity_group(
    settings_page: &adw::PreferencesPage,
    window: &adw::ApplicationWindow,
) -> ConnectivityWidgets {
    let conn_group = adw::PreferencesGroup::builder()
        .title("Connectivity")
        .build();
    settings_page.add(&conn_group);

    let (internal_row, internal_switch, internal_entry) =
        add_url_row(&conn_group, "Internal URL (LAN)", "http://...");
    let (external_row, external_switch, external_entry) =
        add_url_row(&conn_group, "External URL (WAN)", "https://...");
    let api_key_entry = add_api_key_row(&conn_group);
    let test_btn = add_group_button(&conn_group, "Test Connection", None, 12);
    let save_btn = add_group_button(&conn_group, "Save Credentials", Some("suggested-action"), 6);

    add_connectivity_breakpoint(
        window,
        &internal_row,
        &external_row,
        &internal_entry,
        &external_entry,
        &api_key_entry,
    );

    ConnectivityWidgets {
        internal_switch,
        external_switch,
        internal_entry,
        external_entry,
        api_key_entry,
        test_btn,
        save_btn,
    }
}

fn add_url_row(
    group: &adw::PreferencesGroup,
    title: &str,
    placeholder: &str,
) -> (adw::ActionRow, Switch, Entry) {
    let row = adw::ActionRow::builder()
        .title(title)
        .title_lines(1)
        .build();
    let switch = Switch::builder().valign(gtk::Align::Center).build();
    let entry = text_entry(placeholder);
    row.add_prefix(&switch);
    row.add_suffix(&entry);
    group.add(&row);
    (row, switch, entry)
}

fn add_api_key_row(group: &adw::PreferencesGroup) -> PasswordEntry {
    let row = adw::ActionRow::builder().title("API Key").build();
    let entry = PasswordEntry::builder()
        .valign(gtk::Align::Center)
        .width_request(140)
        .max_width_chars(16)
        .hexpand(true)
        .build();
    row.add_suffix(&entry);
    group.add(&row);
    entry
}

fn text_entry(placeholder: &str) -> Entry {
    Entry::builder()
        .placeholder_text(placeholder)
        .valign(gtk::Align::Center)
        .width_request(140)
        .max_width_chars(16)
        .hexpand(true)
        .build()
}

fn add_group_button(
    group: &adw::PreferencesGroup,
    label: &str,
    css_class: Option<&str>,
    margin_top: i32,
) -> Button {
    let mut builder = Button::builder().label(label).margin_top(margin_top);
    if let Some(css_class) = css_class {
        builder = builder.css_classes(vec![css_class.to_string()]);
    }
    let button = builder.build();
    group.add(&button);
    button
}

fn add_connectivity_breakpoint(
    window: &adw::ApplicationWindow,
    internal_row: &adw::ActionRow,
    external_row: &adw::ActionRow,
    internal_entry: &Entry,
    external_entry: &Entry,
    api_key_entry: &PasswordEntry,
) {
    let breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 500sp").expect("valid breakpoint condition"),
    );
    breakpoint.add_setter(internal_row, "title", Some(&"LAN URL".to_value()));
    breakpoint.add_setter(external_row, "title", Some(&"WAN URL".to_value()));
    breakpoint.add_setter(internal_entry, "width-request", Some(&140i32.to_value()));
    breakpoint.add_setter(external_entry, "width-request", Some(&140i32.to_value()));
    breakpoint.add_setter(api_key_entry, "width-request", Some(&140i32.to_value()));
    window.add_breakpoint(breakpoint);
}

/// Warning text for optional API-key permissions the probe found missing, or `None` when
/// nothing is missing. `Unknown` results are ignored: a network blip must not alarm anyone.
pub fn permission_warning(results: &[crate::api_client::PermissionProbeResult]) -> Option<String> {
    use crate::api_client::PermissionState;
    let notes: Vec<String> = results
        .iter()
        .filter(|row| row.state == PermissionState::Missing)
        .map(|row| format!("the key lacks `{}`. {}", row.permission, row.feature_hint))
        .collect();
    if notes.is_empty() {
        None
    } else {
        Some(format!("Note: {}", notes.join("\n\nNote: ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::{PermissionProbeResult, PermissionState};

    fn row(state: PermissionState) -> PermissionProbeResult {
        PermissionProbeResult {
            permission: "stack.read",
            feature_hint: "Stacks will be limited.",
            state,
        }
    }

    #[test]
    fn warns_only_for_missing_permissions() {
        assert_eq!(permission_warning(&[]), None);
        assert_eq!(permission_warning(&[row(PermissionState::Granted)]), None);
        assert_eq!(permission_warning(&[row(PermissionState::Unknown)]), None);
        let warning = permission_warning(&[row(PermissionState::Missing)]).unwrap();
        assert_eq!(
            warning,
            "Note: the key lacks `stack.read`. Stacks will be limited."
        );
    }
}
