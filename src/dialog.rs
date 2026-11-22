//! Generic popup dialogs: a card (like a notification) centered on the
//! focused monitor, with any number of action buttons. The result is the
//! action the user picked, or [`Response::Dismissed`].
//!
//! ```no_run
//! # async fn example(app: &gtk::Application) {
//! use vo1ded_panel::dialog::{Dialog, DialogAction, Response};
//!
//! let dialog = Dialog {
//!     title: "Restart now?".into(),
//!     body: "Updates were installed.".into(),
//!     actions: vec![DialogAction::new("restart", "Restart"), DialogAction::new("later", "Later")],
//!     ..Default::default()
//! };
//! match vo1ded_panel::dialog::run(app, &dialog).await {
//!     Response::Action(key) => println!("picked {key}"),
//!     Response::Dismissed => println!("dismissed"),
//! }
//! # }
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

use crate::services::notifications::{Action, Image, body_markup};
use crate::widgets::card::{Card, card};

pub use crate::services::notifications::Urgency;

/// A button in a [`Dialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogAction {
    /// Returned in [`Response::Action`] when the button is pressed.
    pub key: String,
    pub label: String,
}

impl DialogAction {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

/// What a dialog shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    pub title: String,
    pub body: String,
    /// Interpret `body` as markup (`<b>`, `<i>`, `<u>`, `<a href>`) rather
    /// than plain text.
    pub markup: bool,
    /// Icon name, absolute path or `file://` URI.
    pub icon: Option<String>,
    pub actions: Vec<DialogAction>,
    pub urgency: Urgency,
}

type Callback = Box<dyn FnOnce(Response)>;

/// How the user answered a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Closed with the dismiss button, Escape, or by closing the window.
    Dismissed,
    /// The action with this key was pressed.
    Action(String),
}

/// Shows `dialog` and calls `on_response` once the user answers; the dialog
/// closes itself. Requires a GTK display, i.e. call it from (or after) the
/// application's `activate`, with [`crate::load_assets`] done for styling.
pub fn show(
    app: &gtk::Application,
    dialog: &Dialog,
    on_response: impl FnOnce(Response) + 'static,
) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .css_classes(["notifications", "dialog"])
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("vo1ded-dialog"));
    // Above fullscreen windows; without anchors the compositor centers it on
    // the focused monitor.
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);

    // Answers once: the first response wins, and closes the window.
    let callback: Rc<RefCell<Option<Callback>>> =
        Rc::new(RefCell::new(Some(Box::new(on_response))));
    let respond = {
        let window = window.downgrade();
        let callback = callback.clone();
        Rc::new(move |response: Response| {
            let Some(callback) = callback.borrow_mut().take() else {
                return;
            };
            if let Some(window) = window.upgrade() {
                window.destroy();
            }
            callback(response);
        })
    };

    let image = dialog.icon.as_deref().and_then(Image::from_path);
    let body = if dialog.markup {
        body_markup(&dialog.body)
    } else {
        glib::markup_escape_text(&dialog.body).into()
    };
    let actions: Vec<Action> = dialog
        .actions
        .iter()
        .map(|a| Action {
            key: a.key.clone(),
            label: a.label.clone(),
        })
        .collect();
    let spec = Card {
        title: &dialog.title,
        body_markup: &body,
        image: image.as_ref(),
        actions: &actions,
        action_icons: false,
        urgency: dialog.urgency,
    };
    let (on_action, on_dismiss) = (respond.clone(), respond.clone());
    let content = card(
        &spec,
        move |key| on_action(Response::Action(key.to_owned())),
        move || on_dismiss(Response::Dismissed),
    );
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(8);
    content.set_margin_end(8);
    window.set_child(Some(&content));

    let keys = gtk::EventControllerKey::new();
    let r = respond.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            r(Response::Dismissed);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);

    // Closed some other way, e.g. by the compositor.
    window.connect_destroy(move |_| respond(Response::Dismissed));

    window.present();
    window
}

/// Shows `dialog` and waits for the user's answer. See [`show`].
pub async fn run(app: &gtk::Application, dialog: &Dialog) -> Response {
    let (tx, rx) = async_channel::bounded(1);
    show(app, dialog, move |response| {
        let _ = tx.try_send(response);
    });
    rx.recv().await.unwrap_or(Response::Dismissed)
}
