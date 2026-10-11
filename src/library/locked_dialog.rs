//! Locked Folder sign-in: email, password, and PIN, because Immich only unlocks the
//! folder for a login session. Nothing typed here is stored.

use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::api_client::ImmichApiClient;

use super::LibraryWindowUi;
use super::context_menu::show_alert_dialog;
use super::locked_folder::{begin, end_session, is_valid_pin};

type OnUnlocked = Rc<dyn Fn(Rc<LibraryWindowUi>)>;

const OAUTH_ONLY: &str = "This account signs in with OAuth and has no password, so Mimick \
can't open its Locked Folder. Use the Immich web or mobile app instead.";

const WRONG_ACCOUNT: &str = "That account isn't the one this API key belongs to. Sign in \
with the API key's account.";

/// Check the account can sign in with a password, then show the sign-in form.
pub(super) fn present(ui: Rc<LibraryWindowUi>, on_unlocked: OnUnlocked) {
    glib::MainContext::default().spawn_local(async move {
        let api = &ui.ctx.api_client;
        // With the API key this only tells whether a password and PIN exist; if it
        // fails, assume both and let sign-in report any problem.
        let status = api.auth_status().await.ok();
        if status.as_ref().is_some_and(|s| !s.password) {
            show_alert_dialog(&ui, "Locked Folder Unavailable", OAUTH_ONLY);
            return;
        }
        let email = api
            .fetch_current_user()
            .await
            .map(|user| user.email)
            .unwrap_or_default();
        let has_pin = status.is_none_or(|s| s.pin_code);
        show_form(ui, &email, has_pin, on_unlocked);
    });
}

struct SignInForm {
    email: libadwaita::EntryRow,
    password: libadwaita::PasswordEntryRow,
    pin: libadwaita::PasswordEntryRow,
}

impl SignInForm {
    fn new(email: &str, has_pin: bool) -> Self {
        Self {
            email: libadwaita::EntryRow::builder()
                .title("Email")
                .text(email)
                .build(),
            password: libadwaita::PasswordEntryRow::builder()
                .title("Password")
                .build(),
            pin: libadwaita::PasswordEntryRow::builder()
                .title(if has_pin { "PIN" } else { "New PIN (6 digits)" })
                .input_purpose(gtk::InputPurpose::Pin)
                .build(),
        }
    }

    fn group(&self, has_pin: bool) -> libadwaita::PreferencesGroup {
        let mut description = "Immich only opens the Locked Folder for a signed-in session. \
Your password and PIN aren't saved, and the session ends after 15 minutes, when you lock \
it, or when you close this window."
            .to_string();
        if !has_pin {
            description.push_str("\n\nThis account has no PIN yet; the one you enter becomes it.");
        }
        let group = libadwaita::PreferencesGroup::builder()
            .description(description)
            .build();
        group.add(&self.email);
        group.add(&self.password);
        group.add(&self.pin);
        group
    }
}

fn show_form(ui: Rc<LibraryWindowUi>, email: &str, has_pin: bool, on_unlocked: OnUnlocked) {
    let form = SignInForm::new(email, has_pin);
    let unlock_btn = gtk::Button::builder()
        .label("Unlock")
        .css_classes(["suggested-action"])
        .build();
    let header = libadwaita::HeaderBar::new();
    header.pack_end(&unlock_btn);
    let page = libadwaita::PreferencesPage::new();
    page.add(&form.group(has_pin));
    let toolbar = libadwaita::ToolbarView::builder().content(&page).build();
    toolbar.add_top_bar(&header);
    let dialog = libadwaita::Dialog::builder()
        .title("Unlock Locked Folder")
        .content_width(420)
        .child(&toolbar)
        .build();
    let (ui_unlock, dialog_ref) = (ui.clone(), dialog.clone());
    unlock_btn.connect_clicked(move |btn| {
        let pin = form.pin.text().to_string();
        if !is_valid_pin(&pin) {
            show_alert_dialog(&ui_unlock, "Invalid PIN", "The PIN must be 6 digits.");
            return;
        }
        let (email, password) = (form.email.text(), form.password.text());
        btn.set_sensitive(false);
        let (ui, dialog, btn) = (ui_unlock.clone(), dialog_ref.clone(), btn.clone());
        let on_unlocked = on_unlocked.clone();
        glib::MainContext::default().spawn_local(async move {
            match sign_in(&ui.ctx.api_client, email.trim(), &password, &pin).await {
                Ok((token, pin_expires_at)) => {
                    dialog.close();
                    begin(&ui, token, pin_expires_at.as_deref());
                    on_unlocked(ui.clone());
                }
                Err(err) => {
                    btn.set_sensitive(true);
                    show_alert_dialog(&ui, "Could Not Unlock", &err);
                }
            }
        });
    });
    dialog.present(Some(&ui.window));
}

/// Sign in, confirm it's the API key's account, then unlock. Returns the session token
/// and Immich's unlock expiry; on any failure the session is signed out again.
async fn sign_in(
    api: &Arc<ImmichApiClient>,
    email: &str,
    password: &str,
    pin: &str,
) -> Result<(String, Option<String>), String> {
    let login = api.login(email, password).await?;
    // The session replaces the API key for every request, background sync included,
    // so it must belong to the same account.
    let owner = api.fetch_current_user().await;
    if owner.as_ref().map(|user| user.id.as_str()) != Ok(login.user_id.as_str()) {
        end_session(api, login.access_token);
        return Err(match owner {
            Ok(_) => WRONG_ACCOUNT.to_string(),
            Err(err) => format!("Could not check the API key's account: {err}"),
        });
    }
    api.set_session_token(Some(login.access_token.clone()));
    match unlock(api, pin).await {
        Ok(pin_expires_at) => Ok((login.access_token, pin_expires_at)),
        Err(err) => {
            end_session(api, login.access_token);
            Err(err)
        }
    }
}

/// Set the PIN if the account has none, unlock, and return when the unlock expires.
async fn unlock(api: &ImmichApiClient, pin: &str) -> Result<Option<String>, String> {
    if !api.auth_status().await?.pin_code {
        api.setup_pin(pin).await?;
    }
    api.unlock_session(pin).await?;
    Ok(api.auth_status().await?.pin_expires_at)
}
