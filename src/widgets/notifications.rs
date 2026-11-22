//! Notification popups: hexagons stacked in the top right corner of the
//! focused monitor, or placed at the position a notification asks for.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::services::hyprland;
use crate::services::notifications::{self, Notification};
use crate::widgets::card::{Card, card};

/// Space between stacked notifications and around the stack.
const SPACING: i32 = 8;

/// Builds the card for `notification` (see [`card`]).
pub fn notification_widget(notification: &Notification) -> gtk::Widget {
    let id = notification.id;
    let service = notifications::get();

    // The summary is plain text; prefer it, but fall back to the app name so
    // the card isn't empty.
    let title = if notification.summary.is_empty() {
        &notification.app_name
    } else {
        &notification.summary
    };
    let spec = Card {
        title,
        body_markup: &notification.body,
        image: notification.image.as_ref(),
        actions: &notification.actions,
        action_icons: notification.action_icons,
        urgency: notification.urgency,
    };
    let (s, d) = (service.clone(), service.clone());
    let root = card(&spec, move |key| s.invoke(id, key), move || d.dismiss(id));

    // Clicking the notification invokes its default action, or dismisses
    // it. Buttons and links claim their own clicks.
    let click = gtk::GestureClick::new();
    let (s, has_default) = (service.clone(), notification.has_default_action);
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if has_default {
            s.invoke(id, notifications::DEFAULT_ACTION);
        } else {
            s.dismiss(id);
        }
    });
    root.add_controller(click);

    // Don't expire while the pointer is on it.
    let motion = gtk::EventControllerMotion::new();
    let s = service.clone();
    motion.connect_enter(move |_, _, _| s.hold(id));
    motion.connect_leave(move |_| service.release(id));
    root.add_controller(motion);

    root.upcast()
}

fn layer_window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .css_classes(["notifications"])
        .focusable(false)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("vo1ded-notifications"));
    window.set_layer(Layer::Top);
    window.set_keyboard_mode(KeyboardMode::None);
    window
}

/// The monitor Hyprland has focused, if GDK knows it.
fn focused_monitor() -> Option<gdk::Monitor> {
    let name = hyprland::get().state.get().focused_monitor.clone()?;
    monitors()
        .into_iter()
        .find(|m| m.connector().as_deref() == Some(&name))
}

fn monitors() -> Vec<gdk::Monitor> {
    gdk::Display::default()
        .map(|d| d.monitors().iter::<gdk::Monitor>().flatten().collect())
        .unwrap_or_default()
}

/// Places `window` with its top left corner at screen position (`x`, `y`),
/// on the monitor containing it.
fn place(window: &gtk::ApplicationWindow, (x, y): (i32, i32)) {
    let monitor = monitors()
        .into_iter()
        .find(|m| m.geometry().contains_point(x, y))
        .or_else(focused_monitor);
    let origin = monitor
        .as_ref()
        .map_or((0, 0), |m| (m.geometry().x(), m.geometry().y()));
    window.set_monitor(monitor.as_ref());
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Left, x - origin.0);
    window.set_margin(Edge::Top, y - origin.1);
    // Measure from the monitor's corner, not from below the bar.
    window.set_exclusive_zone(-1);
}

struct Popup {
    notification: Notification,
    widget: gtk::Widget,
    /// Own window for notifications with a position; others are stacked.
    window: Option<gtk::ApplicationWindow>,
}

impl Drop for Popup {
    fn drop(&mut self) {
        if let Some(window) = &self.window {
            window.destroy();
        } else if let Some(parent) = self.widget.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.widget);
        }
    }
}

/// Starts the notification daemon and shows its notifications.
pub fn init(app: &gtk::Application) {
    let stack = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(SPACING)
        .margin_top(SPACING)
        .margin_end(SPACING)
        .margin_bottom(SPACING)
        .margin_start(SPACING)
        .build();
    let window = layer_window(app);
    // Below the bar: its exclusive zone pushes the window down.
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_child(Some(&stack));

    let popups: RefCell<HashMap<u32, Popup>> = RefCell::default();
    let app = app.clone();
    let render = move |list: &Vec<Notification>| {
        let mut popups = popups.borrow_mut();
        // Expired popups live on only in the notification centre.
        let list: Vec<&Notification> = list.iter().filter(|n| n.popup).collect();
        popups.retain(|id, _| list.iter().any(|n| n.id == *id));

        // Notifications are only placed on a monitor when shown, so the
        // stack appears on whichever monitor is focused at the time.
        if !window.is_visible() && list.iter().any(|n| n.position.is_none()) {
            window.set_monitor(focused_monitor().as_ref());
        }

        let mut previous: Option<gtk::Widget> = None;
        for &notification in &list {
            let popup = popups.get(&notification.id);
            if popup.is_none_or(|p| p.notification != *notification) {
                let widget = notification_widget(notification);
                let window = notification.position.map(|position| {
                    let window = layer_window(&app);
                    place(&window, position);
                    window.set_child(Some(&widget));
                    window.present();
                    window
                });
                // Replaces (and so removes) the old popup.
                popups.insert(
                    notification.id,
                    Popup {
                        notification: notification.clone(),
                        widget,
                        window,
                    },
                );
            }
            let popup = &popups[&notification.id];
            if popup.window.is_none() {
                if popup.widget.parent().is_none() {
                    stack.append(&popup.widget);
                }
                stack.reorder_child_after(&popup.widget, previous.as_ref());
                previous = Some(popup.widget.clone());
            }
        }

        window.set_visible(stack.first_child().is_some());
        true
    };
    notifications::get().state.subscribe(render);
}
