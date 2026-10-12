use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{Person, PersonUpdate};

use super::LibraryWindowUi;
use super::context_menu::show_alert_dialog;
use super::dialogs::{TextPrompt, confirm, popover_menu, prompt_text};
use super::explore_view::spawn_person_thumbnail;
use super::state::LibrarySource;

const AVATAR_SIZE: i32 = 120;

pub(super) struct PeopleView {
    pub root: gtk::Box,
    search: gtk::SearchEntry,
    show_hidden: gtk::ToggleButton,
    grid: gtk::FlowBox,
    empty: libadwaita::StatusPage,
    people: RefCell<Vec<Person>>,
}

pub(super) fn build_people_view() -> PeopleView {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search people")
        .hexpand(true)
        .build();
    let show_hidden = gtk::ToggleButton::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text("Show hidden people")
        .build();
    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    toolbar.append(&search);
    toolbar.append(&show_hidden);
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(30)
        .column_spacing(6)
        .row_spacing(12)
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let empty = libadwaita::StatusPage::builder()
        .icon_name("avatar-default-symbolic")
        .vexpand(true)
        .visible(false)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&grid);
    content.append(&empty);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&content)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    root.append(&toolbar);
    root.append(&scroller);
    PeopleView {
        root,
        search,
        show_hidden,
        grid,
        empty,
        people: RefCell::default(),
    }
}

pub(super) fn connect_people_view(ui: Rc<LibraryWindowUi>) {
    let ui_search = ui.clone();
    ui.people
        .search
        .connect_search_changed(move |_| render(&ui_search));
    let ui_hidden = ui.clone();
    ui.people.show_hidden.connect_toggled(move |button| {
        button.set_icon_name(if button.is_active() {
            "view-conceal-symbolic"
        } else {
            "view-reveal-symbolic"
        });
        button.set_tooltip_text(Some(if button.is_active() {
            "Hide hidden people"
        } else {
            "Show hidden people"
        }));
        render(&ui_hidden);
    });
}

pub(super) fn show(ui: &Rc<LibraryWindowUi>) {
    ui.content_stack.set_visible_child_name("people");
    reload(ui.clone());
}

fn reload(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_people(true).await {
            Ok(people) => {
                *ui.people.people.borrow_mut() = people;
                render(&ui);
            }
            Err(err) => show_alert_dialog(
                &ui,
                "Could Not Load People",
                &format!("{err}\n\nYour API key needs the 'person.read' permission."),
            ),
        }
    });
}

fn render(ui: &Rc<LibraryWindowUi>) {
    let view = &ui.people;
    while let Some(child) = view.grid.first_child() {
        view.grid.remove(&child);
    }
    let query = view.search.text().to_lowercase();
    let people = view.people.borrow();
    let shown = visible_people(&people, &query, view.show_hidden.is_active());
    for person in &shown {
        view.grid.append(&person_tile(ui, person));
    }
    let (title, description) = if people.is_empty() {
        (
            "No People",
            "People appear here once Immich has recognized faces in your photos.",
        )
    } else {
        ("No Matches", "No one matches your search.")
    };
    view.empty.set_title(title);
    view.empty.set_description(Some(description));
    view.empty.set_visible(shown.is_empty());
    view.grid.set_visible(!shown.is_empty());
}

fn visible_people<'a>(people: &'a [Person], query: &str, show_hidden: bool) -> Vec<&'a Person> {
    people
        .iter()
        .filter(|person| show_hidden || !person.is_hidden)
        .filter(|person| query.is_empty() || person.name.to_lowercase().contains(query))
        .collect()
}

fn display_name(person: &Person) -> &str {
    if person.name.is_empty() {
        "Unnamed"
    } else {
        &person.name
    }
}

fn avatar(ui: &LibraryWindowUi, person: &Person, size: i32) -> libadwaita::Avatar {
    let avatar = libadwaita::Avatar::new(size, Some(&person.name), true);
    let avatar_for_image = avatar.clone();
    spawn_person_thumbnail(ui.ctx.clone(), person.id.clone(), move |texture| {
        avatar_for_image.set_custom_image(Some(texture))
    });
    avatar
}

fn person_tile(ui: &Rc<LibraryWindowUi>, person: &Person) -> gtk::Overlay {
    let label = gtk::Label::builder()
        .label(display_name(person))
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(14)
        .build();
    if person.name.is_empty() {
        label.add_css_class("dim-label");
    }
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    inner.append(&avatar(ui, person, AVATAR_SIZE));
    inner.append(&label);
    let open = gtk::Button::builder()
        .child(&inner)
        .tooltip_text(format!("Show photos of {}", display_name(person)))
        .css_classes(["flat"])
        .build();
    if person.is_hidden {
        open.set_opacity(0.5);
    }
    let (ui_open, id, name) = (
        ui.clone(),
        person.id.clone(),
        display_name(person).to_string(),
    );
    open.connect_clicked(move |_| {
        super::navigation::navigate_source(
            &ui_open,
            LibrarySource::Person {
                id: id.clone(),
                name: name.clone(),
            },
        );
    });
    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .css_classes(["circular", "osd"])
        .build();
    popover_menu(&menu, person_menu_items(ui, person));
    let tile = gtk::Overlay::builder().child(&open).build();
    tile.add_overlay(&menu);
    tile
}

pub(super) fn person_menu_items(
    ui: &Rc<LibraryWindowUi>,
    person: &Person,
) -> Vec<(String, Box<dyn Fn()>)> {
    let rename_label = if person.name.is_empty() {
        "Add Name…"
    } else {
        "Rename…"
    };
    let hide_label = if person.is_hidden { "Unhide" } else { "Hide" };
    let (ui_rename, p_rename) = (ui.clone(), person.clone());
    let (ui_birthday, p_birthday) = (ui.clone(), person.clone());
    let (ui_hide, p_hide) = (ui.clone(), person.clone());
    let (ui_merge, p_merge) = (ui.clone(), person.clone());
    vec![
        (
            rename_label.to_string(),
            Box::new(move || rename(&ui_rename, &p_rename)),
        ),
        (
            "Set Birthday…".to_string(),
            Box::new(move || edit_birthday(&ui_birthday, &p_birthday)),
        ),
        (
            hide_label.to_string(),
            Box::new(move || {
                let update = PersonUpdate {
                    is_hidden: Some(!p_hide.is_hidden),
                    ..PersonUpdate::default()
                };
                save(&ui_hide, &p_hide.id, update);
            }),
        ),
        (
            "Merge People…".to_string(),
            Box::new(move || present_merge_dialog(&ui_merge, &p_merge)),
        ),
    ]
}

fn save(ui: &Rc<LibraryWindowUi>, person_id: &str, update: PersonUpdate) {
    let (ui, person_id) = (ui.clone(), person_id.to_string());
    glib::MainContext::default().spawn_local(async move {
        if let Err(err) = ui.ctx.api_client.update_person(&person_id, &update).await {
            show_alert_dialog(&ui, "Could Not Update Person", &err);
            return;
        }
        if let Some(person) = ui
            .people
            .people
            .borrow_mut()
            .iter_mut()
            .find(|person| person.id == person_id)
        {
            apply_update(person, &update);
        }
        render(&ui);
        super::source_header::refresh(&ui);
    });
}

fn apply_update(person: &mut Person, update: &PersonUpdate) {
    if let Some(name) = &update.name {
        person.name = name.clone();
    }
    if let Some(hidden) = update.is_hidden {
        person.is_hidden = hidden;
    }
    if let Some(birth_date) = &update.birth_date {
        person.birth_date = birth_date.clone();
    }
}

fn rename(ui: &Rc<LibraryWindowUi>, person: &Person) {
    let ui_save = ui.clone();
    let person_id = person.id.clone();
    prompt_text(
        ui,
        TextPrompt {
            heading: if person.name.is_empty() {
                "Add Name"
            } else {
                "Rename Person"
            },
            body: "This name is used in search and on the People page.",
            initial: &person.name,
            placeholder: "Name",
            accept: "Save",
        },
        move |name| {
            let update = PersonUpdate {
                name: Some(name),
                ..PersonUpdate::default()
            };
            save(&ui_save, &person_id, update);
        },
    );
}

fn edit_birthday(ui: &Rc<LibraryWindowUi>, person: &Person) {
    let calendar = gtk::Calendar::builder().halign(gtk::Align::Center).build();
    if let Some((year, month, day)) = person.birth_date.as_deref().and_then(parse_date) {
        calendar.set_year(year);
        calendar.set_month(month - 1);
        calendar.set_day(day);
    }
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Set Birthday")
        .body(format!(
            "Immich shows {}'s age on photos.",
            display_name(person)
        ))
        .extra_child(&calendar)
        .build();
    dialog.add_response("cancel", "Cancel");
    if person.birth_date.is_some() {
        dialog.add_response("clear", "Remove");
        dialog.set_response_appearance("clear", libadwaita::ResponseAppearance::Destructive);
    }
    dialog.add_response("save", "Save");
    dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let (ui_save, person_id) = (ui.clone(), person.id.clone());
    dialog.connect_response(None, move |_, response| {
        let birth_date = match response {
            "save" => Some(format!(
                "{:04}-{:02}-{:02}",
                calendar.year(),
                calendar.month() + 1,
                calendar.day()
            )),
            "clear" => None,
            _ => return,
        };
        let update = PersonUpdate {
            birth_date: Some(birth_date),
            ..PersonUpdate::default()
        };
        save(&ui_save, &person_id, update);
    });
    dialog.present(Some(&ui.window));
}

fn parse_date(date: &str) -> Option<(i32, i32, i32)> {
    let mut parts = date.get(..10)?.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    Some((year, month, day))
}

fn present_merge_dialog(ui: &Rc<LibraryWindowUi>, target: &Person) {
    if ui.people.people.borrow().is_empty() {
        let (ui, target) = (ui.clone(), target.clone());
        glib::MainContext::default().spawn_local(async move {
            match ui.ctx.api_client.fetch_people(true).await {
                Ok(people) => {
                    *ui.people.people.borrow_mut() = people;
                    present_merge_list(&ui, &target);
                }
                Err(err) => show_alert_dialog(&ui, "Could Not Load People", &err),
            }
        });
        return;
    }
    present_merge_list(ui, target);
}

fn present_merge_list(ui: &Rc<LibraryWindowUi>, target: &Person) {
    let others: Vec<Person> = ui
        .people
        .people
        .borrow()
        .iter()
        .filter(|person| person.id != target.id)
        .cloned()
        .collect();
    let selected: Rc<RefCell<HashSet<String>>> = Rc::default();
    let merge = gtk::Button::builder()
        .label("Merge")
        .sensitive(false)
        .css_classes(["destructive-action"])
        .build();
    let header = libadwaita::HeaderBar::new();
    header.pack_end(&merge);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search people")
        .build();
    let note = gtk::Label::builder()
        .label(format!(
            "Pick the people who are really {}. Their faces move to {} and they're removed. This can't be undone.",
            display_name(target),
            display_name(target)
        ))
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build();
    for person in &others {
        list.append(&merge_row(ui, person, &selected, &merge));
    }
    let query = Rc::new(RefCell::new(String::new()));
    let names: Vec<String> = others.iter().map(|p| p.name.to_lowercase()).collect();
    let query_for_filter = query.clone();
    list.set_filter_func(move |row| {
        let query = query_for_filter.borrow();
        query.is_empty()
            || names
                .get(row.index() as usize)
                .is_some_and(|name| name.contains(query.as_str()))
    });
    let list_for_search = list.clone();
    search.connect_search_changed(move |entry| {
        *query.borrow_mut() = entry.text().to_lowercase();
        list_for_search.invalidate_filter();
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    body.append(&note);
    body.append(&search);
    body.append(&list);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .vexpand(true)
        .child(&body)
        .build();
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));
    let dialog = libadwaita::Dialog::builder()
        .title(format!("Merge Into {}", display_name(target)))
        .content_width(420)
        .content_height(560)
        .child(&toolbar)
        .build();
    let (ui_merge, dialog_for_merge, target) = (ui.clone(), dialog.clone(), target.clone());
    merge.connect_clicked(move |_| {
        let ids: Vec<String> = selected.borrow().iter().cloned().collect();
        let (ui, dialog, target) = (ui_merge.clone(), dialog_for_merge.clone(), target.clone());
        let heading = match ids.len() {
            1 => format!("Merge 1 person into {}?", display_name(&target)),
            n => format!("Merge {n} people into {}?", display_name(&target)),
        };
        confirm(
            &ui_merge,
            &heading,
            "This can't be undone.",
            "Merge",
            move || {
                dialog.close();
                merge_people(ui.clone(), target.id.clone(), ids.clone());
            },
        );
    });
    dialog.present(Some(&ui.window));
}

fn merge_row(
    ui: &LibraryWindowUi,
    person: &Person,
    selected: &Rc<RefCell<HashSet<String>>>,
    merge: &gtk::Button,
) -> libadwaita::ActionRow {
    let check = gtk::CheckButton::builder()
        .valign(gtk::Align::Center)
        .build();
    let row = libadwaita::ActionRow::builder()
        .title(display_name(person))
        .use_markup(false)
        .activatable_widget(&check)
        .build();
    row.add_prefix(&avatar(ui, person, 40));
    row.add_suffix(&check);
    let (selected, merge, id) = (selected.clone(), merge.clone(), person.id.clone());
    check.connect_toggled(move |check| {
        let mut selected = selected.borrow_mut();
        if check.is_active() {
            selected.insert(id.clone());
        } else {
            selected.remove(&id);
        }
        merge.set_sensitive(!selected.is_empty());
    });
    row
}

fn merge_people(ui: Rc<LibraryWindowUi>, target_id: String, ids: Vec<String>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.merge_people(&target_id, &ids).await {
            Ok(()) => {
                super::source_header::refresh(&ui);
                reload(ui);
            }
            Err(err) => show_alert_dialog(&ui, "Could Not Merge People", &err),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: &str, name: &str, hidden: bool) -> Person {
        Person {
            id: id.into(),
            name: name.into(),
            is_hidden: hidden,
            birth_date: None,
        }
    }

    #[test]
    fn visible_people_filters_hidden_and_search() {
        let people = vec![
            person("1", "Ana", false),
            person("2", "Ben", true),
            person("3", "", false),
        ];
        let ids = |shown: Vec<&Person>| shown.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(visible_people(&people, "", false)), ["1", "3"]);
        assert_eq!(ids(visible_people(&people, "", true)), ["1", "2", "3"]);
        assert_eq!(ids(visible_people(&people, "be", true)), ["2"]);
    }

    #[test]
    fn apply_update_changes_only_set_fields() {
        let mut ana = person("1", "Ana", false);
        apply_update(
            &mut ana,
            &PersonUpdate {
                is_hidden: Some(true),
                birth_date: Some(Some("1990-05-04".into())),
                ..PersonUpdate::default()
            },
        );
        assert_eq!(ana.name, "Ana");
        assert!(ana.is_hidden);
        assert_eq!(ana.birth_date.as_deref(), Some("1990-05-04"));
    }

    #[test]
    fn parse_date_reads_iso_dates() {
        assert_eq!(parse_date("1990-05-04"), Some((1990, 5, 4)));
        assert_eq!(parse_date("1990-05-04T00:00:00.000Z"), Some((1990, 5, 4)));
        assert_eq!(parse_date("bad"), None);
    }
}
