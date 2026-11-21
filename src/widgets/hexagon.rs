//! Hexagonal backgrounds. GTK CSS can't clip shapes, so they are drawn with
//! cairo in the CSS `color` of the drawing node.

use gtk::prelude::*;

/// Horizontal size of a hexagon's pointed sides, shared by all hexagons so
/// their angles match.
pub const TIP: f64 = 14.0;

/// Space between a hexagon's outer edge and its content: the tips are
/// narrowest at mid-height, where icons and text sit.
const CONTENT_INSET: i32 = (TIP / 2.0) as i32 + 6;

/// Uses `area`'s CSS `color` as the cairo source.
pub fn set_source(cr: &gtk::cairo::Context, area: &gtk::DrawingArea) {
    let color = area.color();
    cr.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        color.alpha().into(),
    );
}

/// Wraps `content` in a filled hexagon, drawn in the CSS `color` of
/// `css_class`. The hexagon hides along with its content.
pub fn hexagon(content: &impl IsA<gtk::Widget>, css_class: &str) -> gtk::Overlay {
    let shape = gtk::DrawingArea::builder().css_classes([css_class]).build();
    shape.set_draw_func(|area, cr, width, height| {
        let (w, h) = (width as f64, height as f64);
        cr.move_to(0.0, h / 2.0);
        cr.line_to(TIP, 0.0);
        cr.line_to(w - TIP, 0.0);
        cr.line_to(w, h / 2.0);
        cr.line_to(w - TIP, h);
        cr.line_to(TIP, h);
        cr.close_path();
        set_source(cr, area);
        let _ = cr.fill();
    });

    content.set_margin_start(CONTENT_INSET);
    content.set_margin_end(CONTENT_INSET);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&shape));
    overlay.add_overlay(content);
    // Size by the content rather than by the empty drawing area.
    overlay.set_measure_overlay(content, true);
    content
        .bind_property("visible", &overlay, "visible")
        .sync_create()
        .build();
    overlay
}
