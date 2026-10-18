//! Selection-mode wiring and bulk batch operations.
//!
//! Manages multi-select mode toggling, selection-count tracking, and
//! batch actions such as bulk download, bulk delete-to-trash, and
//! bulk add-to-album for the library grid.

use std::cell::Cell;
use std::rc::Rc;

use futures_util::StreamExt;
use glib::clone;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::{SharedLinkTarget, StackResponse, is_forbidden};
use crate::library::asset_object::AssetObject;

use super::LOCAL_ID_PREFIX;
use super::LibraryWindowUi;
use super::download::start_download_group;

pub(super) fn connect_select_mode(ui: Rc<LibraryWindowUi>, select_toggle: gtk::ToggleButton) {
    let selection = ui.grid.selection.clone();
    let bulk_bar = ui.bulk_bar.clone();
    let count_label = ui.bulk_count_label.clone();

    let refresh = {
        let selection = selection.clone();
        let bulk_bar = bulk_bar.clone();
        let count_label = count_label.clone();
        let select_toggle = select_toggle.clone();
        Rc::new(move || {
            let n = selection_count(&selection);
            bulk_bar.set_reveal_child(select_toggle.is_active() && n > 0);
            count_label.set_label(&format!("{} selected", n));
        })
    };

    selection.connect_selection_changed({
        let refresh = refresh.clone();
        move |_, _, _| (*refresh)()
    });

    select_toggle.connect_toggled({
        let selection = selection.clone();
        let refresh = refresh.clone();
        move |toggle| {
            if !toggle.is_active() {
                selection.unselect_all();
            }
            (*refresh)();
        }
    });

    // Ctrl-hold transient checkboxes: pressing Ctrl reveals the selection UI;
    // releasing Ctrl without having selected anything dismisses it again.
    // Ctrl+click then commits a selection (handled separately in grid_view)
    // and the release no longer collapses select mode.
    let transient = Rc::new(Cell::new(false));
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    key_controller.connect_key_pressed({
        let select_toggle = select_toggle.clone();
        let transient = transient.clone();
        move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::Escape && select_toggle.is_active() {
                select_toggle.set_active(false);
                transient.set(false);
                return glib::Propagation::Stop;
            }
            if matches!(keyval, gtk::gdk::Key::Control_L | gtk::gdk::Key::Control_R)
                && !select_toggle.is_active()
            {
                select_toggle.set_active(true);
                transient.set(true);
            }
            glib::Propagation::Proceed
        }
    });
    key_controller.connect_key_released({
        let select_toggle = select_toggle.clone();
        let selection = selection.clone();
        let transient = transient.clone();
        move |_, keyval, _, _| {
            if !matches!(keyval, gtk::gdk::Key::Control_L | gtk::gdk::Key::Control_R)
                || !transient.get()
            {
                return;
            }
            if selection.selection().size() == 0 {
                select_toggle.set_active(false);
            }
            transient.set(false);
        }
    });
    ui.window.add_controller(key_controller);
}

pub(super) fn connect_bulk_actions(
    ui: Rc<LibraryWindowUi>,
    delete_btn: gtk::Button,
    download_btn: gtk::Button,
    clear_btn: gtk::Button,
) {
    clear_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            ui.grid.selection.unselect_all();
            ui.select_toggle.set_active(false);
        }
    ));

    download_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            let items: Vec<AssetObject> = selected_items(&ui)
                .into_iter()
                .filter(|item| !item.id().starts_with(LOCAL_ID_PREFIX))
                .collect();
            if items.is_empty() {
                return;
            }
            // Downloads are saved under their real filenames: thin Main library
            // tiles hydrate first (a no-op for every other source).
            let ui = ui.clone();
            glib::MainContext::default().spawn_local(async move {
                super::hydration::hydrate_objects(&ui, &items).await;
                let downloads = items
                    .iter()
                    .map(|item| (item.id(), item.filename()))
                    .collect();
                start_download_group(ui.clone(), downloads);
            });
        }
    ));

    delete_btn.connect_clicked(clone!(
        #[strong]
        ui,
        move |_| {
            confirm_and_bulk_delete(ui.clone());
        }
    ));

    connect_delete_key(ui);
}

fn connect_delete_key(ui: Rc<LibraryWindowUi>) {
    // On the grid's scroller (not the window) so Delete never fires while typing
    // in the search box or while the lightbox page is showing.
    let delete_key = gtk::EventControllerKey::new();
    delete_key.connect_key_pressed(clone!(
        #[strong]
        ui,
        move |_, keyval, _, _| {
            if keyval != gtk::gdk::Key::Delete || selection_count(&ui.grid.selection) == 0 {
                return glib::Propagation::Proceed;
            }
            if super::trash_view::is_trash_active(&ui) {
                ui.trash.delete_selected.emit_clicked();
            } else {
                confirm_and_bulk_delete(ui.clone());
            }
            glib::Propagation::Stop
        }
    ));
    ui.grid.scrolled.add_controller(delete_key);
}

/// Selection-bar button: create one public link for every selected server asset.
pub(super) fn connect_bulk_share_link(ui: Rc<LibraryWindowUi>, button: &gtk::Button) {
    button.connect_clicked(move |_| {
        let ids: Vec<String> = selected_items(&ui)
            .iter()
            .filter_map(TrashTarget::from_item)
            .map(|target| target.remote_id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let ui_after = ui.clone();
        super::shared_link_dialog::present(
            ui.clone(),
            super::shared_link_dialog::LinkDialogMode::Create(SharedLinkTarget::Assets(ids)),
            Rc::new(move || {
                ui_after.grid.selection.unselect_all();
                ui_after.select_toggle.set_active(false);
            }),
        );
    });
}

pub(super) fn connect_bulk_archive(ui: Rc<LibraryWindowUi>) {
    ui.bulk_archive.clone().connect_clicked(move |_| {
        let archived = !matches!(
            ui.ctx.library_state.lock().source,
            super::state::LibrarySource::Archive
        );
        let ui_done = ui.clone();
        super::archive::set_archived(&ui, selected_items(&ui), archived, move |_| {
            ui_done.grid.selection.unselect_all();
            ui_done.select_toggle.set_active(false);
        });
    });
}

pub(super) fn selection_count(selection: &gtk::MultiSelection) -> u32 {
    let bitset = selection.selection();
    bitset.size() as u32
}

pub(super) fn selected_items(ui: &Rc<LibraryWindowUi>) -> Vec<AssetObject> {
    let bitset = ui.grid.selection.selection();
    let mut out = Vec::new();
    let Some((mut iter, first)) = gtk::BitsetIter::init_first(&bitset) else {
        return out;
    };
    let mut pos = Some(first);
    while let Some(p) = pos {
        // Unloaded Main library slots are placeholders with no id: nothing to act on.
        if let Some(item) = ui.grid.model.item(p).and_downcast::<AssetObject>()
            && !item.id().is_empty()
        {
            out.push(item);
        }
        pos = iter.next();
    }
    out
}
/// A server asset the user asked to move to the Immich trash or archive.
///
/// A Stack tile stands for every member of its stack: archive and delete fan out
/// to all of them (see [`expand_stack_targets`]), while every other action works
/// on the tile's own id, which is the Primary.
pub(super) struct TrashTarget {
    remote_id: String,
    filename: String,
    has_local_copy: bool,
    /// Immich stack id; empty when the tile is not a Stack tile.
    stack_id: String,
    /// Members in the stack (from the bucket's `stack-count`); 0 when not stacked.
    stack_count: u32,
}

impl TrashTarget {
    pub(super) fn remote_id(&self) -> &str {
        &self.remote_id
    }

    /// Strip stack info so the action targets just this asset. Lightbox actions
    /// target the displayed member, never the whole stack (grid actions do the
    /// opposite — see `expand_stack_targets`).
    pub(super) fn member_only(mut self) -> Self {
        self.stack_id.clear();
        self.stack_count = 0;
        self
    }

    /// `None` for local-only items, which have nothing on the server to act on.
    pub(super) fn from_item(item: &AssetObject) -> Option<Self> {
        let id = item.property::<String>("id");
        let remote_id = item.property::<String>("remote-id");
        let remote_id = if remote_id.is_empty() { id } else { remote_id };
        if remote_id.is_empty() || remote_id.starts_with(LOCAL_ID_PREFIX) {
            return None;
        }
        let stack_id = item.stack_id();
        let (stack_id, stack_count) = if item.is_stack() && !stack_id.is_empty() {
            (stack_id, item.stack_count())
        } else {
            (String::new(), 0)
        };
        Some(Self {
            remote_id,
            filename: item.property("filename"),
            has_local_copy: !item.property::<String>("local-path").is_empty(),
            stack_id,
            stack_count,
        })
    }

    fn is_stack(&self) -> bool {
        !self.stack_id.is_empty()
    }

    /// Files this target covers on the server: the whole stack, or the one asset.
    fn file_count(&self) -> u32 {
        if self.is_stack() {
            self.stack_count.max(1)
        } else {
            1
        }
    }
}

/// Why a stack fan-out was aborted before any mutation was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FanOutError {
    /// `fetch_stack` was refused: the API key lacks `stack.read`.
    MissingStackPermission,
    /// Any other `fetch_stack` failure.
    Failed(String),
}

/// Shown when a stack could not be expanded for lack of `stack.read`.
const STACK_PERMISSION_MESSAGE: &str =
    "Archiving/deleting a stack requires the `stack.read` API permission.";

/// Every member id of a fetched stack, Primary first, without duplicates.
fn stack_member_ids(stack: &StackResponse) -> Vec<String> {
    let mut ids = vec![stack.primary_asset_id.clone()];
    for asset in &stack.assets {
        if !ids.contains(&asset.id) {
            ids.push(asset.id.clone());
        }
    }
    ids
}

/// Combine the targets' own ids with the fetched members of their stacks.
///
/// No partial action: if *any* stack failed to resolve, nothing is returned.
/// A missing permission outranks other failures because the user can act on it.
fn combine_fan_out(
    own_ids: Vec<String>,
    stacks: Vec<Result<Vec<String>, String>>,
) -> Result<Vec<String>, FanOutError> {
    let mut failure: Option<FanOutError> = None;
    let mut ids = own_ids;
    for stack in stacks {
        match stack {
            Ok(members) => {
                for id in members {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            Err(err) if is_forbidden(&err) => return Err(FanOutError::MissingStackPermission),
            Err(err) => {
                failure.get_or_insert(FanOutError::Failed(err));
            }
        }
    }
    match failure {
        Some(err) => Err(err),
        None => Ok(ids),
    }
}

/// Server ids that archive/delete of `targets` must touch: each target itself plus,
/// for Stack tiles, every other stack member (resolved with `fetch_stack` first, so
/// a failure aborts before anything is changed).
pub(super) async fn expand_stack_targets(
    ui: &LibraryWindowUi,
    targets: &[TrashTarget],
) -> Result<Vec<String>, FanOutError> {
    /// Concurrent `GET /stacks/{id}` requests.
    const STACK_CONCURRENCY: usize = 6;

    let own_ids: Vec<String> = targets.iter().map(|t| t.remote_id.clone()).collect();
    let mut stack_ids: Vec<&str> = targets
        .iter()
        .filter(|t| t.is_stack())
        .map(|t| t.stack_id.as_str())
        .collect();
    stack_ids.sort_unstable();
    stack_ids.dedup();
    if stack_ids.is_empty() {
        return Ok(own_ids);
    }
    let fetched: Vec<Result<Vec<String>, String>> = futures_util::stream::iter(stack_ids)
        .map(|stack_id| async move {
            ui.ctx
                .api_client
                .fetch_stack(stack_id)
                .await
                .map(|stack| stack_member_ids(&stack))
        })
        .buffer_unordered(STACK_CONCURRENCY)
        .collect()
        .await;
    combine_fan_out(own_ids, fetched)
}

/// Tell the user why a stack fan-out was aborted. `generic_heading` titles any
/// non-permission failure.
pub(super) fn report_fan_out_error(ui: &LibraryWindowUi, err: &FanOutError, generic_heading: &str) {
    match err {
        FanOutError::MissingStackPermission => super::context_menu::show_alert_dialog(
            ui,
            "Missing API Permissions",
            STACK_PERMISSION_MESSAGE,
        ),
        FanOutError::Failed(err) => {
            log::error!("Could not resolve stack members: {err}");
            super::context_menu::show_alert_dialog(
                ui,
                generic_heading,
                &format!("Could not look up the stack's photos: {err}"),
            );
        }
    }
}

/// " (including 2 stacks, 9 files total)" for a bulk heading, or empty when no
/// target is a stack.
fn stack_scope(targets: &[TrashTarget]) -> String {
    let stacks = targets.iter().filter(|t| t.is_stack()).count();
    if stacks == 0 {
        return String::new();
    }
    let files: u32 = targets.iter().map(TrashTarget::file_count).sum();
    let noun = if stacks == 1 { "stack" } else { "stacks" };
    format!(" (including {stacks} {noun}, {files} files total)")
}

/// Heading and body for the trash confirmation dialog.
fn trash_dialog_text(
    targets: &[TrashTarget],
    skipped_local: usize,
    trash: bool,
) -> (String, String) {
    // Trash works on IDs only, so thin (unhydrated) Main library tiles have no
    // filename to quote.
    let (verb, suffix) = if trash {
        ("Move", "to trash")
    } else {
        ("Delete", "permanently")
    };
    let heading = match targets {
        [single] if single.is_stack() => {
            format!("{verb} stack of {} {suffix}?", single.file_count())
        }
        [single] if single.filename.is_empty() => format!("{verb} this item {suffix}?"),
        [single] => format!("{verb} “{}” {suffix}?", single.filename),
        _ => format!(
            "{verb} {} items{} {suffix}?",
            targets.len(),
            stack_scope(targets)
        ),
    };
    let mut body = if trash {
        "Items can be restored from the Immich trash.".to_string()
    } else {
        "The Immich trash is turned off, so this can't be undone.".to_string()
    };
    if targets.iter().any(|t| t.has_local_copy) {
        body.push_str(" Local copies in your watch folders are kept.");
    }
    if skipped_local > 0 {
        body.push_str(&format!(
            "\n\n{} local-only item(s) are not on the server and will be skipped.",
            skipped_local
        ));
    }
    (heading, body)
}

/// Ask before moving `targets` to the Immich trash; `on_trashed` runs after the server
/// confirms. Grid semantics: a Stack tile trashes every member (fan-out).
pub(super) fn confirm_trash(
    ui: Rc<LibraryWindowUi>,
    targets: Vec<TrashTarget>,
    skipped_local: usize,
    on_trashed: impl Fn() + 'static,
) {
    confirm_trash_inner(ui, targets, skipped_local, true, on_trashed);
}

/// [`confirm_trash`] for the lightbox: targets the displayed member only — never
/// the whole stack — and can leave the grid untouched (`remove_from_grid` false),
/// since a stack member is not a grid tile and dropping the Primary's tile would
/// shift the positions under the open viewer.
pub(super) fn confirm_trash_with(
    ui: Rc<LibraryWindowUi>,
    targets: Vec<TrashTarget>,
    skipped_local: usize,
    remove_from_grid: bool,
    on_trashed: impl Fn() + 'static,
) {
    let targets: Vec<TrashTarget> = targets.into_iter().map(TrashTarget::member_only).collect();
    confirm_trash_inner(ui, targets, skipped_local, remove_from_grid, on_trashed);
}

fn confirm_trash_inner(
    ui: Rc<LibraryWindowUi>,
    targets: Vec<TrashTarget>,
    skipped_local: usize,
    remove_from_grid: bool,
    on_trashed: impl Fn() + 'static,
) {
    if targets.is_empty() {
        return;
    }
    let trash = ui.features.get().trash;
    let (heading, body) = trash_dialog_text(&targets, skipped_local, trash);
    let dialog = libadwaita::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", if trash { "Move to trash" } else { "Delete" });
    dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let targets = Rc::new(targets);
    let on_trashed = Rc::new(on_trashed);
    let ui_for_choice = ui.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let ui = ui_for_choice.clone();
        let targets = targets.clone();
        let on_trashed = on_trashed.clone();
        glib::MainContext::default().spawn_local(async move {
            // Resolve every stack before trashing anything: no partial action.
            let ids = match expand_stack_targets(&ui, &targets).await {
                Ok(ids) => ids,
                Err(err) => {
                    report_fan_out_error(&ui, &err, "Could Not Move to Trash");
                    return;
                }
            };
            match trash_assets(&ui, &ids, remove_from_grid).await {
                Ok(()) => on_trashed(),
                Err(err) => {
                    log::error!("Move to trash failed: {}", err);
                    super::context_menu::show_alert_dialog(
                        &ui,
                        "Could Not Move to Trash",
                        &format!("Immich rejected the request: {err}"),
                    );
                }
            }
        });
    });
    dialog.present(Some(&ui.window));
}

/// Trash on the server, then drop the items from the grid (or just refresh counts).
async fn trash_assets(
    ui: &Rc<LibraryWindowUi>,
    ids: &[String],
    remove_from_grid: bool,
) -> Result<(), String> {
    ui.ctx.api_client.delete_assets(ids).await?;
    if remove_from_grid {
        remove_assets_from_grid(ui, ids);
    } else {
        super::refresh_library_after_mutation(ui.clone(), false);
    }
    Ok(())
}

/// Drop assets from the grid in place, so open views (like the lightbox) keep their
/// position instead of reloading from the first page; then refresh album/stat counts.
pub(super) fn remove_assets_from_grid(ui: &Rc<LibraryWindowUi>, ids: &[String]) {
    if super::main_library::remove_ids(ui, ids) {
        super::refresh_library_after_mutation(ui.clone(), false);
        return;
    }
    let assets = {
        let mut state = ui.ctx.library_state.lock();
        state.assets.retain(|asset| !ids.contains(&asset.id));
        state.assets.clone()
    };
    // Reset outside the lock: items_changed handlers may read the library state.
    ui.grid.model.reset(&ui.ctx, &assets);
    super::refresh_library_after_mutation(ui.clone(), false);
}

fn confirm_and_bulk_delete(ui: Rc<LibraryWindowUi>) {
    let items = selected_items(&ui);
    let targets: Vec<TrashTarget> = items.iter().filter_map(TrashTarget::from_item).collect();
    let skipped_local = items.len() - targets.len();
    let ui_after = ui.clone();
    confirm_trash(ui, targets, skipped_local, move || {
        ui_after.grid.selection.unselect_all();
        ui_after.select_toggle.set_active(false);
    });
}

#[cfg(test)]
mod tests {
    use super::{FanOutError, TrashTarget, combine_fan_out, stack_member_ids, trash_dialog_text};
    use crate::api_client::{STACK_FORBIDDEN, StackResponse};
    use crate::library::LOCAL_ID_PREFIX;
    use crate::library::asset_object::{AssetInit, AssetObject};
    use glib::prelude::ObjectExt;

    fn remote_item(id: &str) -> AssetObject {
        AssetObject::new(AssetInit {
            id,
            filename: "IMG_1.jpg",
            mime_type: "image/jpeg",
            created_at: "2024-01-15T14:25:15.000Z",
            asset_type: "IMAGE",
            sync_state: 0,
            thumbhash: None,
            width: 0,
            height: 0,
            is_favorite: false,
            is_archived: false,
        })
    }

    #[test]
    fn trash_target_uses_remote_id_for_server_assets() {
        let target = TrashTarget::from_item(&remote_item("remote-1")).expect("server asset");
        assert_eq!(target.remote_id(), "remote-1");
        assert_eq!(target.filename, "IMG_1.jpg");
        assert!(!target.has_local_copy);
    }

    #[test]
    fn trash_target_notes_a_local_copy() {
        let item = remote_item("remote-1");
        item.set_property("local-path", "/home/user/Pictures/IMG_1.jpg");
        let target = TrashTarget::from_item(&item).expect("server asset");
        assert!(target.has_local_copy);
    }

    #[test]
    fn trash_target_skips_local_only_assets() {
        let path = "/home/user/Pictures/IMG_2.jpg";
        let item = AssetObject::new_local(
            &format!("{LOCAL_ID_PREFIX}{path}"),
            "IMG_2.jpg",
            "image/jpeg",
            "2024-01-15T14:25:15.000Z",
            "IMAGE",
            path,
        );
        assert!(TrashTarget::from_item(&item).is_none());
    }

    fn target(name: &str, has_local_copy: bool) -> TrashTarget {
        TrashTarget {
            remote_id: format!("id-{name}"),
            filename: name.to_string(),
            has_local_copy,
            stack_id: String::new(),
            stack_count: 0,
        }
    }

    #[test]
    fn single_item_heading_names_the_file() {
        let (heading, body) = trash_dialog_text(&[target("IMG_1.jpg", false)], 0, true);
        assert_eq!(heading, "Move “IMG_1.jpg” to trash?");
        assert_eq!(body, "Items can be restored from the Immich trash.");
    }

    #[test]
    fn single_thin_tile_heading_does_not_quote_an_empty_name() {
        let (heading, _) = trash_dialog_text(&[target("", false)], 0, true);
        assert_eq!(heading, "Move this item to trash?");
        let (heading, _) = trash_dialog_text(&[target("", false)], 0, false);
        assert_eq!(heading, "Delete this item permanently?");
    }

    #[test]
    fn body_mentions_kept_local_copies_and_skipped_items() {
        let targets = [target("a.jpg", true), target("b.jpg", false)];
        let (heading, body) = trash_dialog_text(&targets, 2, true);
        assert_eq!(heading, "Move 2 items to trash?");
        assert!(body.contains("Local copies in your watch folders are kept."));
        assert!(body.contains("2 local-only item(s)"));
    }

    #[test]
    fn without_trash_the_dialog_warns_deletion_is_permanent() {
        let (heading, body) = trash_dialog_text(&[target("IMG_1.jpg", false)], 0, false);
        assert_eq!(heading, "Delete “IMG_1.jpg” permanently?");
        assert!(body.contains("can't be undone"));
    }

    fn stack_target(name: &str, count: u32) -> TrashTarget {
        TrashTarget {
            stack_id: format!("stack-{name}"),
            stack_count: count,
            ..target(name, false)
        }
    }

    fn stack_item(id: &str, count: u32) -> AssetObject {
        let item = remote_item(id);
        item.set_property("stack-id", format!("stack-{id}"));
        item.set_property("stack-count", count);
        item
    }

    #[test]
    fn stack_tile_target_carries_its_stack() {
        let target = TrashTarget::from_item(&stack_item("primary-1", 4)).expect("server asset");
        // Favorite/download/copy use `remote_id`: still the Primary, not a member list.
        assert_eq!(target.remote_id(), "primary-1");
        assert!(target.is_stack());
        assert_eq!(target.file_count(), 4);
    }

    #[test]
    fn plain_tile_is_not_a_stack_target() {
        let target = TrashTarget::from_item(&remote_item("a")).expect("server asset");
        assert!(!target.is_stack());
        assert_eq!(target.file_count(), 1);
    }

    #[test]
    fn single_stack_heading_names_the_member_count() {
        let (heading, _) = trash_dialog_text(&[stack_target("a", 4)], 0, true);
        assert_eq!(heading, "Move stack of 4 to trash?");
        let (heading, _) = trash_dialog_text(&[stack_target("a", 4)], 0, false);
        assert_eq!(heading, "Delete stack of 4 permanently?");
    }

    #[test]
    fn mixed_selection_heading_discloses_stacks_and_total_files() {
        let mut targets: Vec<TrashTarget> =
            (0..5).map(|i| target(&format!("p{i}"), false)).collect();
        targets.push(stack_target("s1", 5));
        targets.push(stack_target("s2", 6));
        let (heading, _) = trash_dialog_text(&targets, 0, true);
        assert_eq!(
            heading,
            "Move 7 items (including 2 stacks, 16 files total) to trash?"
        );
        let (heading, _) = trash_dialog_text(&targets, 0, false);
        assert_eq!(
            heading,
            "Delete 7 items (including 2 stacks, 16 files total) permanently?"
        );
    }

    #[test]
    fn one_stack_among_many_uses_the_singular() {
        let targets = [target("a", false), stack_target("s", 3)];
        let (heading, _) = trash_dialog_text(&targets, 0, true);
        assert_eq!(
            heading,
            "Move 2 items (including 1 stack, 4 files total) to trash?"
        );
    }

    #[test]
    fn selections_without_stacks_keep_the_plain_heading() {
        let targets = [target("a", false), target("b", false)];
        let (heading, _) = trash_dialog_text(&targets, 0, true);
        assert_eq!(heading, "Move 2 items to trash?");
    }

    fn stack_response(primary: &str, members: &[&str]) -> StackResponse {
        let assets: Vec<serde_json::Value> = members
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "originalFileName": "x.jpg",
                    "originalMimeType": "image/jpeg",
                    "fileCreatedAt": "2024-01-15T14:25:15.000Z",
                    "type": "IMAGE"
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "id": "s",
            "primaryAssetId": primary,
            "assets": assets
        }))
        .unwrap()
    }

    #[test]
    fn stack_members_list_primary_first_without_duplicates() {
        let stack = stack_response("b", &["a", "b", "c"]);
        assert_eq!(stack_member_ids(&stack), ["b", "a", "c"]);
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn fan_out_adds_members_to_the_targets_own_ids() {
        let result = combine_fan_out(
            ids(&["solo", "p1"]),
            vec![Ok(ids(&["p1", "m1", "m2"])), Ok(ids(&["p2", "m3"]))],
        );
        assert_eq!(result, Ok(ids(&["solo", "p1", "m1", "m2", "p2", "m3"])));
    }

    #[test]
    fn fan_out_without_stacks_is_just_the_targets() {
        assert_eq!(
            combine_fan_out(ids(&["a", "b"]), vec![]),
            Ok(ids(&["a", "b"]))
        );
    }

    #[test]
    fn fan_out_aborts_on_403_even_if_other_stacks_resolved() {
        let result = combine_fan_out(
            ids(&["p1", "p2"]),
            vec![Ok(ids(&["p1", "m1"])), Err(STACK_FORBIDDEN.to_string())],
        );
        assert_eq!(result, Err(FanOutError::MissingStackPermission));
    }

    #[test]
    fn fan_out_aborts_on_other_failures_with_the_error() {
        let result = combine_fan_out(
            ids(&["p1", "p2"]),
            vec![
                Err("HTTP 500 Internal Server Error".to_string()),
                Ok(ids(&["p2"])),
            ],
        );
        assert_eq!(
            result,
            Err(FanOutError::Failed("HTTP 500 Internal Server Error".into()))
        );
    }

    #[test]
    fn missing_permission_outranks_other_failures() {
        let result = combine_fan_out(
            ids(&["p1", "p2"]),
            vec![Err("timeout".to_string()), Err(STACK_FORBIDDEN.to_string())],
        );
        assert_eq!(result, Err(FanOutError::MissingStackPermission));
    }
}
