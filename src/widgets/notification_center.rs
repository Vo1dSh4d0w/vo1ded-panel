//! The notification centre: a bar button opening a list of every open
//! notification, including those whose popups have expired.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::prelude::*;

use crate::services::notifications::{self, Notification};
use crate::widgets::hexagon::{hex_row, hexagon};
use crate::widgets::notifications::notification_widget;

/// Height above which the list scrolls.
const MAX_LIST_HEIGHT: i32 = 700;

/// The notification as the centre shows it; whether it is also a popup
/// doesn't matter here, so popups expiring don't rebuild the list.
fn listed(notification: &Notification) -> Notification {
    Notification {
        popup: false,
        ..notification.clone()
    }
}

fn popover_content() -> gtk::Box {
    let title = gtk::Label::builder()
        .label("Notifications")
        .css_classes(["notification-center-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let clear = gtk::Button::builder().label("Clear all").build();
    clear.connect_clicked(|_| notifications::get().dismiss_all());
    let clear = hex_row(&[clear], Some(0), true, "notification-action");

    let header = gtk::Box::builder().spacing(8).margin_start(8).build();
    header.append(&title);
    header.append(&clear);

    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .propagate_natural_width(true)
        .max_content_height(MAX_LIST_HEIGHT)
        .build();

    let empty = gtk::Label::builder()
        .label("No notifications")
        .css_classes(["notification-center-empty"])
        .margin_top(16)
        .margin_bottom(16)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    content.append(&header);
    content.append(&scroller);
    content.append(&empty);

    let rows: RefCell<HashMap<u32, (Notification, gtk::Widget)>> = RefCell::default();
    let weak = content.downgrade();
    notifications::get().state.subscribe(move |state| {
        if weak.upgrade().is_none() {
            return false;
        }
        let mut rows = rows.borrow_mut();
        rows.retain(|id, (_, widget)| {
            let keep = state.iter().any(|n| n.id == *id);
            if !keep {
                list.remove(widget);
            }
            keep
        });

        // Newest first, like the popups.
        let mut previous: Option<gtk::Widget> = None;
        for notification in state {
            let view = listed(notification);
            let stale = rows.get(&notification.id).is_none_or(|(n, _)| *n != view);
            if stale {
                let widget = notification_widget(notification);
                if let Some((_, old)) = rows.insert(notification.id, (view, widget.clone())) {
                    list.remove(&old);
                }
                list.append(&widget);
            }
            let widget = &rows[&notification.id].1;
            list.reorder_child_after(widget, previous.as_ref());
            previous = Some(widget.clone());
        }

        let has_any = !state.is_empty();
        scroller.set_visible(has_any);
        clear.set_visible(has_any);
        empty.set_visible(!has_any);
        true
    });

    content
}

/// The bar button opening the notification centre, on a hexagonal
/// background. Shows a filled bell and the count while there are
/// notifications.
pub fn notification_center() -> gtk::Overlay {
    let icon = gtk::Image::new();
    let count = gtk::Label::builder()
        .css_classes(["notification-center-count"])
        .build();
    let content = gtk::Box::builder().spacing(4).build();
    content.append(&icon);
    content.append(&count);

    let weak = content.downgrade();
    notifications::get().state.subscribe(move |state| {
        if weak.upgrade().is_none() {
            return false;
        }
        let n = state.len();
        icon.set_icon_name(Some(if n > 0 {
            "vo1ded-bell-symbolic"
        } else {
            "vo1ded-bell-outline-symbolic"
        }));
        count.set_label(&n.to_string());
        count.set_visible(n > 0);
        true
    });

    let popover = gtk::Popover::new();
    popover.set_child(Some(&popover_content()));
    // The button is at the screen edge; open leftwards so the popover keeps
    // its position when it grows or shrinks (see the mixer's popover).
    popover.set_halign(gtk::Align::End);
    // Everything is in the centre now; don't show it twice.
    popover.connect_show(|_| notifications::get().expire_all());

    let button = gtk::MenuButton::new();
    button.add_css_class("notification-center-button");
    button.set_child(Some(&content));
    button.set_popover(Some(&popover));

    hexagon(&button, "notification-center-background")
}
