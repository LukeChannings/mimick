//! Locked Folder: assets Immich hides behind the account PIN.
//!
//! Immich won't unlock it for an API key, so `locked_dialog` signs in with email, password,
//! and PIN. The session token lives only in memory and ends after at most 15 minutes, on
//! "Lock", or when the window closes; then every request goes back to the API key.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use gtk::prelude::*;

use crate::api_client::ImmichApiClient;

use super::LibraryWindowUi;
use super::actions::remove_assets_from_grid;
use super::context_menu::show_alert_dialog;
use super::state::LibrarySource;

/// Immich's own unlock lasts 15 minutes; Mimick never keeps it longer.
const MAX_UNLOCK_MINUTES: i64 = 15;

/// The signed-in session behind an unlocked Locked Folder.
pub(super) struct LockedSession {
    token: String,
    /// Auto-lock timer; `None` once it has fired.
    timer: Option<glib::SourceId>,
}

/// Bar above the grid while the Locked Folder is open: unlock status and "Lock".
pub(super) struct LockedControls {
    pub bar: gtk::Revealer,
    status: gtk::Label,
    lock: gtk::Button,
}

pub(super) fn build_locked_controls() -> LockedControls {
    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    let lock = gtk::Button::builder().label("Lock").build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    inner.append(&status);
    inner.append(&lock);
    let bar = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .reveal_child(false)
        .child(&inner)
        .build();
    LockedControls { bar, status, lock }
}

/// When the unlock ends: Immich's PIN expiry, capped at 15 minutes from `now`.
fn unlock_deadline(pin_expires_at: Option<&str>, now: DateTime<Utc>) -> DateTime<Utc> {
    let cap = now + chrono::Duration::minutes(MAX_UNLOCK_MINUTES);
    pin_expires_at
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc).min(cap))
        .unwrap_or(cap)
}

/// Time left until `deadline`; zero once it has passed.
fn time_left(deadline: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (deadline - now).to_std().unwrap_or_default()
}

/// Immich PINs are exactly six digits.
pub(super) fn is_valid_pin(pin: &str) -> bool {
    pin.len() == 6 && pin.chars().all(|c| c.is_ascii_digit())
}

pub(super) fn is_active(ui: &LibraryWindowUi) -> bool {
    matches!(
        ui.ctx.library_state.lock().source,
        LibrarySource::LockedFolder
    )
}

pub(super) fn set_locked_mode(ui: &LibraryWindowUi, active: bool) {
    ui.locked.bar.set_reveal_child(active);
}

/// Run `then` with the Locked Folder unlocked, signing in first when needed.
fn with_unlocked(ui: &Rc<LibraryWindowUi>, then: impl Fn(Rc<LibraryWindowUi>) + 'static) {
    if ui.locked_session.borrow().is_some() {
        then(ui.clone());
    } else {
        super::locked_dialog::present(ui.clone(), Rc::new(then));
    }
}

/// Sidebar entry: open the Locked Folder, signing in first when needed.
pub(super) fn open(ui: &Rc<LibraryWindowUi>) {
    if ui.locked_session.borrow().is_some() {
        super::controls::sidebar_dispatch(ui.clone(), LibrarySource::LockedFolder);
        return;
    }
    // Unselect so the row can be clicked again if sign-in is cancelled; re-selecting
    // after unlock re-enters `open`, which then takes the branch above.
    ui.sidebar.fixed_list.unselect_all();
    super::locked_dialog::present(
        ui.clone(),
        Rc::new(|ui| {
            super::controls::sidebar_dispatch(ui.clone(), LibrarySource::LockedFolder);
            super::select_fixed_row(&ui.sidebar.fixed_list, "locked");
        }),
    );
}

/// Store the session from a successful sign-in and schedule the auto-lock.
pub(super) fn begin(ui: &Rc<LibraryWindowUi>, token: String, pin_expires_at: Option<&str>) {
    let now = Utc::now();
    let deadline = unlock_deadline(pin_expires_at, now);
    let weak = Rc::downgrade(ui);
    let timer = glib::timeout_add_local_once(time_left(deadline, now), move || {
        if let Some(ui) = weak.upgrade() {
            // A fired source is already gone, so `lock` must not remove it again.
            if let Some(session) = ui.locked_session.borrow_mut().as_mut() {
                session.timer = None;
            }
            lock(&ui);
        }
    });
    let until = deadline.with_timezone(&chrono::Local).format("%H:%M");
    ui.locked.status.set_label(&format!(
        "Unlocked until {until}. Thumbnails here are kept in memory only."
    ));
    *ui.locked_session.borrow_mut() = Some(LockedSession {
        token,
        timer: Some(timer),
    });
}

/// Send every request back to the API key now, then sign `token` out in the background.
pub(super) fn end_session(api: &Arc<ImmichApiClient>, token: String) {
    api.set_session_token(None);
    let api = api.clone();
    glib::MainContext::default().spawn_local(async move {
        if let Err(err) = api.logout_session(&token).await {
            log::warn!("Locked Folder sign-out failed: {err}");
        }
    });
}

/// Lock the folder: end the session, forget locked thumbnails, and leave the view.
pub(super) fn lock(ui: &Rc<LibraryWindowUi>) {
    let Some(session) = ui.locked_session.borrow_mut().take() else {
        return;
    };
    if let Some(timer) = session.timer {
        timer.remove();
    }
    end_session(&ui.ctx.api_client, session.token);
    ui.ctx.thumbnail_cache.clear_memory();
    let in_locked = {
        let mut state = ui.ctx.library_state.lock();
        state
            .nav_history
            .retain(|source| *source != LibrarySource::LockedFolder);
        state.source == LibrarySource::LockedFolder
    };
    if in_locked {
        super::controls::sidebar_dispatch(ui.clone(), LibrarySource::Timeline);
        super::select_fixed_row(&ui.sidebar.fixed_list, "photos");
    }
}

/// Wire "Lock", and lock when the window closes.
pub(super) fn connect_locked_controls(ui: &Rc<LibraryWindowUi>) {
    let weak = Rc::downgrade(ui);
    ui.locked.lock.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            lock(&ui);
        }
    });
    let weak = Rc::downgrade(ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            lock(&ui);
        }
        glib::Propagation::Proceed
    });
}

/// Move assets out of the Locked Folder to the timeline; `on_done` runs after.
pub(super) fn move_out(ui: Rc<LibraryWindowUi>, ids: Vec<String>, on_done: Rc<dyn Fn()>) {
    set_visibility(ui, ids, "timeline", on_done);
}

/// Move assets into the Locked Folder, signing in first when needed; `on_done` runs after.
pub(super) fn move_in(ui: &Rc<LibraryWindowUi>, ids: Vec<String>, on_done: Rc<dyn Fn()>) {
    with_unlocked(ui, move |ui| {
        set_visibility(ui, ids.clone(), "locked", on_done.clone())
    });
}

fn set_visibility(
    ui: Rc<LibraryWindowUi>,
    ids: Vec<String>,
    visibility: &'static str,
    on_done: Rc<dyn Fn()>,
) {
    glib::MainContext::default().spawn_local(async move {
        match ui
            .ctx
            .api_client
            .set_assets_visibility(&ids, visibility)
            .await
        {
            Ok(()) => {
                remove_assets_from_grid(&ui, &ids);
                on_done();
            }
            Err(err) => show_alert_dialog(&ui, "Could Not Move", &err),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-11T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn unlock_deadline_uses_server_expiry_capped_at_15_minutes() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let early = unlock_deadline(Some("2026-10-11T10:05:00.000Z"), now());
        assert_eq!(early, at("2026-10-11T10:05:00Z"));
        let late = unlock_deadline(Some("2026-10-11T11:00:00.000Z"), now());
        assert_eq!(late, at("2026-10-11T10:15:00Z"));
        assert_eq!(unlock_deadline(None, now()), at("2026-10-11T10:15:00Z"));
        assert_eq!(
            unlock_deadline(Some("garbage"), now()),
            at("2026-10-11T10:15:00Z")
        );
    }

    #[test]
    fn time_left_is_zero_once_expired() {
        let later = now() + chrono::Duration::minutes(2);
        assert_eq!(time_left(later, now()), Duration::from_secs(120));
        assert_eq!(time_left(now(), later), Duration::ZERO);
    }

    #[test]
    fn pin_must_be_six_digits() {
        assert!(is_valid_pin("012345"));
        assert!(!is_valid_pin("12345"));
        assert!(!is_valid_pin("12345a"));
        assert!(!is_valid_pin("1234567"));
    }
}
