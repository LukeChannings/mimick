use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::Tag;

use super::LibraryWindowUi;
use super::asset_tags::color_dot;
use super::context_menu::show_alert_dialog;
use super::dialogs::{TextPrompt, confirm, popover_menu, prompt_text};
use super::state::LibrarySource;

const PRESET_COLORS: [&str; 10] = [
    "#e11d48", "#f97316", "#eab308", "#22c55e", "#14b8a6", "#0ea5e9", "#6366f1", "#a855f7",
    "#ec4899", "#64748b",
];

pub(super) struct TagsView {
    pub root: gtk::Box,
    breadcrumbs: gtk::Box,
    breadcrumb_scroller: gtk::ScrolledWindow,
    new_tag: gtk::Button,
    list: gtk::ListBox,
    empty: libadwaita::StatusPage,
    tags: RefCell<Vec<Tag>>,
    path: RefCell<Option<String>>,
}

pub(super) fn build_tags_view() -> TagsView {
    let breadcrumbs = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(2)
        .build();
    let breadcrumb_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .child(&breadcrumbs)
        .build();
    let new_tag = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New tag")
        .build();
    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    toolbar.append(&breadcrumb_scroller);
    toolbar.append(&new_tag);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build();
    let empty = libadwaita::StatusPage::builder()
        .icon_name("mimick-tag-symbolic")
        .vexpand(true)
        .visible(false)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&list);
    content.append(&empty);
    let clamp = libadwaita::Clamp::builder()
        .maximum_size(820)
        .child(&content)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    root.append(&toolbar);
    root.append(&scroller);
    TagsView {
        root,
        breadcrumbs,
        breadcrumb_scroller,
        new_tag,
        list,
        empty,
        tags: RefCell::default(),
        path: RefCell::default(),
    }
}

pub(super) fn connect_tags_view(ui: Rc<LibraryWindowUi>) {
    let ui_new = ui.clone();
    ui.tags
        .new_tag
        .connect_clicked(move |_| create_tag(&ui_new));
}

pub(super) fn show(ui: &Rc<LibraryWindowUi>) {
    ui.content_stack.set_visible_child_name("tags");
    reload(ui.clone());
}

fn reload(ui: Rc<LibraryWindowUi>) {
    glib::MainContext::default().spawn_local(async move {
        match ui.ctx.api_client.fetch_tags().await {
            Ok(tags) => {
                *ui.tags.tags.borrow_mut() = tags;
                render(&ui);
            }
            Err(err) => show_alert_dialog(
                &ui,
                "Could Not Load Tags",
                &format!("{err}\n\nYour API key needs the 'tag.read' permission."),
            ),
        }
    });
}

fn open_path(ui: &Rc<LibraryWindowUi>, path: Option<String>) {
    *ui.tags.path.borrow_mut() = path;
    render(ui);
}

fn render(ui: &Rc<LibraryWindowUi>) {
    let view = &ui.tags;
    let tags = view.tags.borrow().clone();
    let path = view
        .path
        .borrow()
        .clone()
        .filter(|path| tags.iter().any(|tag| &tag.value == path));
    *view.path.borrow_mut() = path.clone();
    render_breadcrumbs(ui, path.as_deref());

    view.list.remove_all();
    let children = children_of(&tags, path.as_deref());
    for tag in &children {
        view.list
            .append(&tag_row(ui, tag, child_count(&tags, &tag.value)));
    }
    let (title, description) = match &path {
        None => (
            "No Tags",
            "Create a tag with the + button, or tag photos from the lightbox details pane.",
        ),
        Some(_) => ("No Tags Inside", "Use the + button to add one here."),
    };
    view.empty.set_title(title);
    view.empty.set_description(Some(description));
    view.empty.set_visible(children.is_empty());
    view.list.set_visible(!children.is_empty());
}

fn render_breadcrumbs(ui: &Rc<LibraryWindowUi>, path: Option<&str>) {
    let view = &ui.tags;
    while let Some(child) = view.breadcrumbs.first_child() {
        view.breadcrumbs.remove(&child);
    }
    let crumbs = breadcrumbs(path);
    let last = crumbs.len() - 1;
    for (index, (label, target)) in crumbs.into_iter().enumerate() {
        if index > 0 {
            let separator = gtk::Label::builder()
                .label("/")
                .css_classes(["dim-label"])
                .build();
            view.breadcrumbs.append(&separator);
        }
        let button = gtk::Button::builder()
            .label(&label)
            .css_classes(["flat"])
            .build();
        if index == last {
            button.add_css_class("heading");
        }
        let ui = ui.clone();
        button.connect_clicked(move |_| open_path(&ui, target.clone()));
        view.breadcrumbs.append(&button);
    }
    let adjustment = view.breadcrumb_scroller.hadjustment();
    glib::idle_add_local_once(move || adjustment.set_value(adjustment.upper()));
}

fn tag_row(ui: &Rc<LibraryWindowUi>, tag: &Tag, children: usize) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title(&tag.name)
        .use_markup(false)
        .title_lines(1)
        .activatable(true)
        .tooltip_text(format!("Show photos tagged {}", tag.value))
        .build();
    if children > 0 {
        row.set_subtitle(&match children {
            1 => "1 tag inside".to_string(),
            n => format!("{n} tags inside"),
        });
    }
    row.add_prefix(&color_dot(tag.color.as_deref(), 12));
    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    popover_menu(&menu, tag_menu_items(ui, tag));
    row.add_suffix(&menu);
    if children > 0 {
        let browse = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text(format!("Open {}", tag.name))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (ui_browse, value) = (ui.clone(), tag.value.clone());
        browse.connect_clicked(move |_| open_path(&ui_browse, Some(value.clone())));
        row.add_suffix(&browse);
    }
    let (ui_open, id, value) = (ui.clone(), tag.id.clone(), tag.value.clone());
    row.connect_activated(move |_| {
        super::controls::sidebar_dispatch(
            ui_open.clone(),
            LibrarySource::Tag {
                id: id.clone(),
                name: value.clone(),
            },
        );
    });
    row
}

fn tag_menu_items(ui: &Rc<LibraryWindowUi>, tag: &Tag) -> Vec<(String, Box<dyn Fn()>)> {
    let (ui_rename, t_rename) = (ui.clone(), tag.clone());
    let (ui_color, t_color) = (ui.clone(), tag.clone());
    let (ui_delete, t_delete) = (ui.clone(), tag.clone());
    vec![
        (
            "Rename…".to_string(),
            Box::new(move || rename_tag(&ui_rename, &t_rename)),
        ),
        (
            "Change Color…".to_string(),
            Box::new(move || pick_color(&ui_color, &t_color)),
        ),
        (
            "Delete…".to_string(),
            Box::new(move || delete_tag(&ui_delete, &t_delete)),
        ),
    ]
}

fn run(
    ui: &Rc<LibraryWindowUi>,
    heading: &'static str,
    task: impl Future<Output = Result<(), String>> + 'static,
) {
    let ui = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        match task.await {
            Ok(()) => reload(ui),
            Err(err) => show_alert_dialog(&ui, heading, &err),
        }
    });
}

fn create_tag(ui: &Rc<LibraryWindowUi>) {
    let parent = ui.tags.path.borrow().clone();
    let body = match &parent {
        Some(parent) => format!("The tag is created inside {parent}."),
        None => "Use “Parent/Child” to create a tag inside another.".to_string(),
    };
    let ui_create = ui.clone();
    prompt_text(
        ui,
        TextPrompt {
            heading: "New Tag",
            body: &body,
            initial: "",
            placeholder: "Tag name",
            accept: "Create",
        },
        move |name| {
            let value = match &parent {
                Some(parent) => format!("{parent}/{name}"),
                None => name,
            };
            let api = ui_create.ctx.api_client.clone();
            run(&ui_create, "Could Not Create Tag", async move {
                api.create_tag(&value).await.map(|_| ())
            });
        },
    );
}

fn rename_tag(ui: &Rc<LibraryWindowUi>, tag: &Tag) {
    let (ui_rename, tag_id) = (ui.clone(), tag.id.clone());
    prompt_text(
        ui,
        TextPrompt {
            heading: "Rename Tag",
            body: "Photos keep the tag under its new name.",
            initial: &tag.name,
            placeholder: "Tag name",
            accept: "Rename",
        },
        move |name| {
            if name.contains('/') {
                show_alert_dialog(
                    &ui_rename,
                    "Could Not Rename Tag",
                    "Tag names can't contain “/”.",
                );
                return;
            }
            let (api, tag_id) = (ui_rename.ctx.api_client.clone(), tag_id.clone());
            run(&ui_rename, "Could Not Rename Tag", async move {
                api.rename_tag(&tag_id, &name).await
            });
        },
    );
}

fn delete_tag(ui: &Rc<LibraryWindowUi>, tag: &Tag) {
    let (ui_delete, tag_id) = (ui.clone(), tag.id.clone());
    confirm(
        ui,
        &format!("Delete “{}”?", tag.name),
        "The tag is removed from every photo. This can't be undone.",
        "Delete",
        move || {
            let (api, tag_id) = (ui_delete.ctx.api_client.clone(), tag_id.clone());
            run(&ui_delete, "Could Not Delete Tag", async move {
                api.delete_tag(&tag_id).await
            });
        },
    );
}

fn pick_color(ui: &Rc<LibraryWindowUi>, tag: &Tag) {
    let chosen = Rc::new(RefCell::new(tag.color.clone().unwrap_or_default()));
    let swatches = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(5)
        .max_children_per_line(5)
        .halign(gtk::Align::Center)
        .build();
    let buttons: Rc<Vec<gtk::ToggleButton>> = Rc::new(
        PRESET_COLORS
            .iter()
            .map(|color| {
                gtk::ToggleButton::builder()
                    .child(&color_dot(Some(color), 22))
                    .tooltip_text(*color)
                    .css_classes(["flat", "circular"])
                    .active(chosen.borrow().eq_ignore_ascii_case(color))
                    .build()
            })
            .collect(),
    );
    for (swatch, color) in buttons.iter().zip(PRESET_COLORS) {
        let (chosen, buttons_for_toggle) = (chosen.clone(), Rc::downgrade(&buttons));
        swatch.connect_toggled(move |swatch| {
            if !swatch.is_active() {
                return;
            }
            *chosen.borrow_mut() = color.to_string();
            if let Some(buttons) = buttons_for_toggle.upgrade() {
                for other in buttons.iter().filter(|other| *other != swatch) {
                    other.set_active(false);
                }
            }
        });
        swatches.append(swatch);
    }
    let custom = gtk::ColorDialogButton::builder()
        .dialog(&gtk::ColorDialog::builder().with_alpha(false).build())
        .halign(gtk::Align::Center)
        .tooltip_text("Custom color")
        .build();
    if let Some(rgba) = tag
        .color
        .as_deref()
        .and_then(|c| gtk::gdk::RGBA::parse(c).ok())
    {
        custom.set_rgba(&rgba);
    }
    let (chosen_custom, buttons_custom) = (chosen.clone(), buttons.clone());
    custom.connect_rgba_notify(move |button| {
        for swatch in buttons_custom.iter() {
            swatch.set_active(false);
        }
        *chosen_custom.borrow_mut() = rgba_hex(&button.rgba());
    });
    let custom_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    custom_row.append(&gtk::Label::new(Some("Custom")));
    custom_row.append(&custom);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    body.append(&swatches);
    body.append(&custom_row);
    let dialog = libadwaita::AlertDialog::builder()
        .heading(format!("Color for “{}”", tag.name))
        .extra_child(&body)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.set_response_appearance("save", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let (ui_save, tag_id) = (ui.clone(), tag.id.clone());
    dialog.connect_response(None, move |_, response| {
        let color = chosen.borrow().clone();
        if response != "save" || color.is_empty() {
            return;
        }
        let (api, tag_id) = (ui_save.ctx.api_client.clone(), tag_id.clone());
        run(&ui_save, "Could Not Change Color", async move {
            api.set_tag_color(&tag_id, &color).await
        });
    });
    dialog.present(Some(&ui.window));
}

fn rgba_hex(rgba: &gtk::gdk::RGBA) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.red()),
        channel(rgba.green()),
        channel(rgba.blue())
    )
}

fn parent_path(value: &str) -> Option<&str> {
    value.rsplit_once('/').map(|(parent, _)| parent)
}

fn children_of<'a>(tags: &'a [Tag], parent: Option<&str>) -> Vec<&'a Tag> {
    tags.iter()
        .filter(|tag| parent_path(&tag.value) == parent)
        .collect()
}

fn child_count(tags: &[Tag], value: &str) -> usize {
    tags.iter()
        .filter(|tag| parent_path(&tag.value) == Some(value))
        .count()
}

fn breadcrumbs(path: Option<&str>) -> Vec<(String, Option<String>)> {
    let mut crumbs = vec![("All Tags".to_string(), None)];
    if let Some(path) = path {
        let mut prefix = String::new();
        for segment in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            crumbs.push((segment.to_string(), Some(prefix.clone())));
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(value: &str) -> Tag {
        Tag {
            id: value.into(),
            name: value.rsplit('/').next().unwrap().into(),
            value: value.into(),
            color: None,
        }
    }

    #[test]
    fn children_of_lists_one_level() {
        let tags = vec![
            tag("Trips"),
            tag("Trips/Japan"),
            tag("Trips/Japan/Kyoto"),
            tag("Food"),
        ];
        let values = |found: Vec<&Tag>| found.iter().map(|t| t.value.clone()).collect::<Vec<_>>();
        assert_eq!(values(children_of(&tags, None)), ["Trips", "Food"]);
        assert_eq!(values(children_of(&tags, Some("Trips"))), ["Trips/Japan"]);
        assert_eq!(child_count(&tags, "Trips"), 1);
        assert_eq!(child_count(&tags, "Food"), 0);
    }

    #[test]
    fn breadcrumbs_build_each_ancestor_path() {
        assert_eq!(breadcrumbs(None), [("All Tags".to_string(), None)]);
        assert_eq!(
            breadcrumbs(Some("Trips/Japan")),
            [
                ("All Tags".to_string(), None),
                ("Trips".to_string(), Some("Trips".to_string())),
                ("Japan".to_string(), Some("Trips/Japan".to_string())),
            ]
        );
    }

    #[test]
    fn rgba_hex_formats_lowercase_hex() {
        let rgba = gtk::gdk::RGBA::new(1.0, 0.0, 0.5, 1.0);
        assert_eq!(rgba_hex(&rgba), "#ff0080");
    }
}
