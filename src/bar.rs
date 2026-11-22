use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::widgets;

pub fn bar(app: &gtk::Application, monitor: &gdk::Monitor) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .name("vo1ded-panel")
        .css_classes(["bar"])
        .focusable(false)
        .build();

    window.init_layer_shell();
    window.set_namespace(Some("vo1ded-panel"));
    window.set_monitor(Some(monitor));
    window.set_layer(Layer::Top);
    window.auto_exclusive_zone_enable();
    window.set_keyboard_mode(KeyboardMode::None);
    for edge in [Edge::Top, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }

    // Runs off the left screen edge, so only the right side is slanted.
    let start = region(&widgets::media::media(), false, true);
    start.set_hexpand(true);
    start.set_halign(gtk::Align::Start);

    let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    center.append(&widgets::workspaces::workspaces(
        monitor.connector().map(Into::into),
    ));
    let center = region(&center, true, true);
    center.set_hexpand(true);
    center.set_halign(gtk::Align::Center);

    let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    end.append(&widgets::tray::tray());
    end.append(&widgets::wireplumber::wireplumber());
    end.append(&widgets::time::time());
    end.append(&widgets::notification_center::notification_center());
    // Runs off the right screen edge, so only the left side is slanted.
    let end = region(&end, true, false);
    end.set_hexpand(true);
    end.set_halign(gtk::Align::End);

    // GtkCenterBox's CSS node is called "box" by default; the stylesheet
    // targets `window.bar > centerbox`.
    let centerbox = gtk::glib::Object::builder::<gtk::CenterBox>()
        .property("css-name", "centerbox")
        .build();
    centerbox.set_start_widget(Some(&start));
    centerbox.set_center_widget(Some(&center));
    centerbox.set_end_widget(Some(&end));

    window.set_child(Some(&centerbox));
    window.present();
    window
}

/// Horizontal size of a region's slanted sides.
const SLANT: i32 = 46;
/// Radius of the rounded corners at the bottom of each slant.
const CORNER_RADIUS: f64 = 4.0;

/// Wraps `content` in a trapezoid hanging from the top screen edge, wider at
/// the top. GTK CSS can't clip shapes, so the background is drawn with cairo
/// in the shape's CSS `color` (see `.bar-region` in `bar.scss`).
fn region(content: &impl IsA<gtk::Widget>, slant_left: bool, slant_right: bool) -> gtk::Overlay {
    let shape = gtk::DrawingArea::builder()
        .css_classes(["bar-region"])
        .build();
    shape.set_draw_func(move |area, cr, width, height| {
        let (w, h) = (width as f64, height as f64);
        let left = if slant_left { SLANT as f64 } else { 0.0 };
        let right = if slant_right { SLANT as f64 } else { 0.0 };

        // Only the corners at the foot of a slant are rounded; unslanted
        // sides sit flush against the screen edge.
        cr.move_to(0.0, 0.0);
        if slant_left {
            rounded_corner(cr, (0.0, 0.0), (left, h), (w - right, h));
        } else {
            cr.line_to(0.0, h);
        }
        if slant_right {
            rounded_corner(cr, (left, h), (w - right, h), (w, 0.0));
        } else {
            cr.line_to(w, h);
        }
        cr.line_to(w, 0.0);
        cr.close_path();

        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        let _ = cr.fill();
    });

    content.add_css_class("bar-region-content");
    content.set_margin_start(if slant_left { SLANT } else { 0 });
    content.set_margin_end(if slant_right { SLANT } else { 0 });

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&shape));
    overlay.add_overlay(content);
    // Size the region by its content rather than by the empty drawing area.
    overlay.set_measure_overlay(content, true);
    // A region whose content hides itself (e.g. no media playing) disappears.
    content
        .bind_property("visible", &overlay, "visible")
        .sync_create()
        .build();
    overlay
}

/// Continues the path from `prev` towards `corner`, rounding it off before
/// heading on to `next`.
fn rounded_corner(
    cr: &gtk::cairo::Context,
    prev: (f64, f64),
    corner: (f64, f64),
    next: (f64, f64),
) {
    let towards = |from: (f64, f64), to: (f64, f64)| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len = dx.hypot(dy);
        if len == 0.0 {
            return corner;
        }
        let r = CORNER_RADIUS.min(len / 2.0);
        (corner.0 + dx / len * r, corner.1 + dy / len * r)
    };
    let before = towards(corner, prev);
    let after = towards(corner, next);
    cr.line_to(before.0, before.1);
    cr.curve_to(corner.0, corner.1, corner.0, corner.1, after.0, after.1);
}
