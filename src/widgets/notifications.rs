//! Notification popups: hexagons stacked in the top right corner of the
//! focused monitor, or placed at the position a notification asks for.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::services::hyprland;
use crate::services::notifications::{self, Image, Notification, Urgency};
use crate::widgets::hexagon::{LINE_WIDTH, TIP, hex_row, hexagon, set_source};

/// Width of a notification.
const WIDTH: i32 = 400;
/// Horizontal size of a notification's pointed sides. Notifications vary in
/// height, so the angle varies; the width is fixed to keep content clear.
const NOTIFICATION_TIP: f64 = 24.0;
/// Height of the hexagon holding the image; its width makes it regular.
const IMAGE_HEIGHT: i32 = 48;
/// Size of themed icons inside the image hexagon.
const ICON_SIZE: i32 = 32;
/// Lines of body text shown before it scrolls.
const BODY_LINES: i32 = 6;
/// Space between stacked notifications and around the stack.
const SPACING: i32 = 8;

/// Traces a hexagon pointed left and right, filling `width` x `height` at
/// (`x`, `y`) with tips `tip` wide.
fn hexagon_path(cr: &gtk::cairo::Context, (x, y, width, height): (f64, f64, f64, f64), tip: f64) {
    cr.move_to(x, y + height / 2.0);
    cr.line_to(x + tip, y);
    cr.line_to(x + width - tip, y);
    cr.line_to(x + width, y + height / 2.0);
    cr.line_to(x + width - tip, y + height);
    cr.line_to(x + tip, y + height);
    cr.close_path();
}

/// The notification's outline, inset by half the line width so it isn't
/// clipped.
fn outline_path(area: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
    let half = LINE_WIDTH / 2.0;
    let rect = (
        half,
        half,
        area.width() as f64 - LINE_WIDTH,
        area.height() as f64 - LINE_WIDTH,
    );
    hexagon_path(cr, rect, NOTIFICATION_TIP);
}

/// Loads a notification image as a texture; `None` for themed icons or
/// unreadable files.
fn texture(image: &Image) -> Option<gdk::Texture> {
    match image {
        Image::Data {
            width,
            height,
            rowstride,
            has_alpha,
            data,
        } => {
            let format = if *has_alpha {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            };
            Some(gdk::MemoryTexture::new(*width, *height, format, data, *rowstride as usize).upcast())
        }
        Image::File(path) => gdk::Texture::from_file(&gio::File::for_path(path))
            .inspect_err(|e| eprintln!("notifications: {path}: {e}"))
            .ok(),
        Image::Icon(_) => None,
    }
}

/// Paints `texture` over the image hexagon, scaled to cover it and clipped
/// to its shape.
fn draw_texture(cr: &gtk::cairo::Context, texture: &gdk::Texture, width: f64, height: f64) {
    let (tw, th) = (texture.width(), texture.height());
    let stride = tw as usize * 4;
    let mut data = vec![0; stride * th as usize];
    // Downloads as cairo's ARGB32.
    texture.download(&mut data, stride);
    let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(
        data,
        gtk::cairo::Format::ARgb32,
        tw,
        th,
        stride as i32,
    ) else {
        return;
    };
    hexagon_path(cr, (0.0, 0.0, width, height), TIP);
    cr.clip();
    let scale = (width / tw as f64).max(height / th as f64);
    cr.translate(
        (width - tw as f64 * scale) / 2.0,
        (height - th as f64 * scale) / 2.0,
    );
    cr.scale(scale, scale);
    if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
        cr.source().set_filter(gtk::cairo::Filter::Good);
        let _ = cr.paint();
    }
}

/// The notification's image in a hexagon: pictures fill and are clipped to
/// it, themed icons sit on a filled hexagon.
fn image_hexagon(image: &Image) -> Option<gtk::Widget> {
    if let Image::Icon(name) = image {
        let icon = gtk::Image::builder()
            .icon_name(name)
            .pixel_size(ICON_SIZE)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let shape = gtk::DrawingArea::builder()
            .css_classes(["notification-image-background"])
            .build();
        shape.set_draw_func(|area, cr, width, height| {
            hexagon_path(cr, (0.0, 0.0, width as f64, height as f64), TIP);
            set_source(cr, area);
            let _ = cr.fill();
        });
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&shape));
        overlay.add_overlay(&icon);
        return Some(overlay.upcast());
    }

    let texture = texture(image)?;
    let area = gtk::DrawingArea::new();
    area.set_draw_func(move |_, cr, width, height| {
        draw_texture(cr, &texture, width as f64, height as f64);
    });
    Some(area.upcast())
}

/// Builds the popup for `notification`:
///
/// ```text
///    ____________________________ _
///   / ___                       </>\
///  < /img\  summary                 >
///   \\___/  body                   /
///    ‾‾‾‾‾‾‾ <action> <action> ‾‾‾‾
/// ```
pub fn notification_widget(notification: &Notification) -> gtk::Widget {
    let id = notification.id;
    let service = notifications::get();

    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(2)
        .margin_start(NOTIFICATION_TIP as i32)
        .margin_end(NOTIFICATION_TIP as i32)
        .margin_top(8)
        .margin_bottom(8)
        .build();

    let image = notification.image.as_ref().and_then(image_hexagon);
    if let Some(image) = &image {
        // A regular hexagon, whose tips match the bar's (`TIP`) at this
        // height.
        let width = (IMAGE_HEIGHT as f64 * 2.0 / 3f64.sqrt()).round() as i32;
        image.set_size_request(width, IMAGE_HEIGHT);
        image.set_valign(gtk::Align::Center);
        image.set_halign(gtk::Align::Start);
        grid.attach(image, 0, 0, 1, 2);
    }

    // The summary is plain text; prefer it, but fall back to the app name so
    // the popup isn't empty.
    let title = if notification.summary.is_empty() {
        &notification.app_name
    } else {
        &notification.summary
    };
    let summary = gtk::Label::builder()
        .label(title)
        .css_classes(["notification-summary"])
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        // Natural width comes from the window, not the text.
        .max_width_chars(1)
        // Clear of the dismiss button.
        .margin_end(32)
        .build();
    summary.set_tooltip_text(Some(title));
    grid.attach(&summary, 1, 0, 1, 1);

    if !notification.body.is_empty() {
        let body = gtk::Label::builder()
            .css_classes(["notification-body"])
            .use_markup(true)
            .label(&notification.body)
            .xalign(0.0)
            .yalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .max_width_chars(1)
            .build();
        // GTK's default handler opens links with the window as parent, which
        // on Wayland means exporting a handle to it; layer surfaces can't be
        // exported, and the compositor kills the connection. Launch them
        // without a parent instead.
        body.connect_activate_link(|_, uri| {
            if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>)
            {
                eprintln!("notifications: {uri}: {e}");
            }
            glib::Propagation::Stop
        });
        // Pango's line limit applies per paragraph, so it can't cap a body
        // with many line breaks; cap the height instead and scroll the rest.
        let metrics = body.pango_context().metrics(None, None);
        let line_height = (metrics.height() / pango::SCALE).max(1);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&body)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(BODY_LINES * line_height)
            .hexpand(true)
            .build();
        grid.attach(&scroller, 1, 1, 1, 1);
    }

    if !notification.actions.is_empty() {
        let actions = gtk::Box::builder()
            .spacing(4)
            .halign(gtk::Align::Center)
            .margin_top(4)
            .build();
        for action in &notification.actions {
            let button = if notification.action_icons {
                gtk::Button::builder()
                    .child(&gtk::Image::from_icon_name(&action.key))
                    .tooltip_text(&action.label)
                    .build()
            } else {
                gtk::Button::builder()
                    .child(&gtk::Label::new(Some(&action.label)))
                    .build()
            };
            button.add_css_class("notification-action");
            let (service, key) = (service.clone(), action.key.clone());
            button.connect_clicked(move |_| service.invoke(id, &key));
            actions.append(&hex_row(&[button], Some(0), true, "notification-action"));
        }
        grid.attach(&actions, 0, 2, 2, 1);
    }

    let dismiss = gtk::Button::builder()
        .icon_name("vo1ded-window-close-symbolic")
        .css_classes(["notification-dismiss"])
        .tooltip_text("Dismiss")
        .build();
    let s = service.clone();
    dismiss.connect_clicked(move |_| s.dismiss(id));
    let dismiss = hexagon(&dismiss, "notification-dismiss-background");
    dismiss.set_halign(gtk::Align::End);
    dismiss.set_valign(gtk::Align::Start);
    // Its right tip stays inside the upper right slant.
    dismiss.set_margin_end(NOTIFICATION_TIP as i32);
    dismiss.set_margin_top(4);

    let fill = gtk::DrawingArea::builder()
        .css_classes(["notification-fill"])
        .build();
    fill.set_draw_func(|area, cr, _, _| {
        outline_path(area, cr);
        set_source(cr, area);
        let _ = cr.fill();
    });
    let outline = gtk::DrawingArea::builder()
        .css_classes(["notification-outline"])
        .can_target(false)
        .build();
    outline.set_draw_func(|area, cr, _, _| {
        outline_path(area, cr);
        cr.set_line_width(LINE_WIDTH);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        set_source(cr, area);
        let _ = cr.stroke();
    });

    let root = gtk::Overlay::builder()
        .css_classes([
            "notification",
            match notification.urgency {
                Urgency::Low => "notification-low",
                Urgency::Normal => "notification-normal",
                Urgency::Critical => "notification-critical",
            },
        ])
        .width_request(WIDTH)
        .build();
    root.set_child(Some(&fill));
    root.add_overlay(&outline);
    root.add_overlay(&grid);
    root.set_measure_overlay(&grid, true);
    root.add_overlay(&dismiss);

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
    monitors().into_iter().find(|m| m.connector().as_deref() == Some(&name))
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
