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

/// Line width of hexagon row outlines.
pub const LINE_WIDTH: f64 = 1.0;
/// Space between a row button's edge and its content: just enough to clear
/// the chevron tip reaching into the button (half a tip plus half the line),
/// plus a 1px gap.
const ROW_CONTENT_INSET: i32 = (TIP / 2.0 + LINE_WIDTH / 2.0) as i32 + 2;

/// One chevron per button boundary: (top, tip, bottom) points.
type Chevron = [(f64, f64); 3];

/// Chevrons at the boundaries of the buttons in `row`, in `area`'s
/// coordinates. Chevrons point away from the `center` button: those left of
/// it point left, the rest point right.
fn chevrons(area: &gtk::DrawingArea, row: &gtk::Box, center: Option<usize>) -> Vec<Chevron> {
    let mut bounds = Vec::new();
    let mut child = row.first_child();
    while let Some(widget) = child {
        bounds.extend(widget.compute_bounds(area));
        child = widget.next_sibling();
    }
    let Some(first) = bounds.first() else {
        return Vec::new();
    };

    let edges = std::iter::once(first.x())
        .chain(bounds.iter().map(|b| b.x() + b.width()))
        .map(f64::from);
    let height = area.height() as f64;
    let (top, bottom) = (LINE_WIDTH / 2.0, height - LINE_WIDTH / 2.0);
    let half = TIP / 2.0;
    edges
        .enumerate()
        .map(|(k, x)| {
            let dir = if center.is_some_and(|c| k <= c) {
                -1.0
            } else {
                1.0
            };
            [
                (x - dir * half, top),
                (x + dir * half, height / 2.0),
                (x - dir * half, bottom),
            ]
        })
        .collect()
}

/// A row of buttons outlined as interlocking hexagon halves; the `center`
/// button is a full hexagon, filled if `fill_center`. The fill and outline
/// are drawn in the CSS `color` of `.{class}-fill` / `.{class}-outline`.
/// Buttons get the `hex-row-button` class, which makes them transparent.
pub fn hex_row(
    buttons: &[gtk::Button],
    center: Option<usize>,
    fill_center: bool,
    class: &str,
) -> gtk::Overlay {
    let row = gtk::Box::builder()
        // Room for the outermost tips.
        .margin_start((TIP / 2.0 + LINE_WIDTH) as i32)
        .margin_end((TIP / 2.0 + LINE_WIDTH) as i32)
        .build();
    for button in buttons {
        button.add_css_class("hex-row-button");
        // Keep the content clear of the chevrons drawn between buttons.
        if let Some(child) = button.child() {
            child.set_margin_start(ROW_CONTENT_INSET);
            child.set_margin_end(ROW_CONTENT_INSET);
        }
        row.append(button);
    }

    let fill = gtk::DrawingArea::builder()
        .css_classes([format!("{class}-fill")])
        .build();
    let row_weak = row.downgrade();
    fill.set_draw_func(move |area, cr, _, _| {
        let (Some(row), Some(c)) = (row_weak.upgrade(), center.filter(|_| fill_center)) else {
            return;
        };
        let chevrons = chevrons(area, &row, center);
        let (Some(left), Some(right)) = (chevrons.get(c), chevrons.get(c + 1)) else {
            return;
        };
        for &(x, y) in left.iter().chain(right.iter().rev()) {
            cr.line_to(x, y);
        }
        cr.close_path();
        set_source(cr, area);
        let _ = cr.fill();
    });

    let outline = gtk::DrawingArea::builder()
        .css_classes([format!("{class}-outline")])
        .can_target(false)
        .build();
    let row_weak = row.downgrade();
    outline.set_draw_func(move |area, cr, _, _| {
        let Some(row) = row_weak.upgrade() else {
            return;
        };
        let chevrons = chevrons(area, &row, center);
        let (Some(first), Some(last)) = (chevrons.first(), chevrons.last()) else {
            return;
        };
        // Top and bottom edges, then every chevron. Stroking it all as one
        // path keeps overlapping segments from doubling up translucent colors.
        for (from, to) in [(first[0], last[0]), (first[2], last[2])] {
            cr.move_to(from.0, from.1);
            cr.line_to(to.0, to.1);
        }
        for [top, tip, bottom] in &chevrons {
            cr.move_to(top.0, top.1);
            cr.line_to(tip.0, tip.1);
            cr.line_to(bottom.0, bottom.1);
        }
        cr.set_line_width(LINE_WIDTH);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        set_source(cr, area);
        let _ = cr.stroke();
    });

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&fill));
    overlay.add_overlay(&outline);
    overlay.add_overlay(&row);
    overlay.set_measure_overlay(&row, true);
    overlay
}
