use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, pango};

use crate::services::audio::{self, Endpoint, Kind, State};
use crate::store::Store;
use crate::widgets::hexagon::{LINE_WIDTH, hexagon, set_source};

const MAX_VOLUME: f64 = 1.5;

thread_local! {
    /// Whether the mixer shows every device and stream, shared by all bars.
    static EXPANDED: Rc<Store<bool>> = Store::new(false);
}

fn expanded() -> Rc<Store<bool>> {
    EXPANDED.with(Rc::clone)
}

/// Horizontal size of a mixer row's pointed sides. Wider than the bar's
/// hexagons since rows are about twice as tall.
const ROW_TIP: f64 = 24.0;
/// Width of the bands in the lower corners holding the mute button and the
/// volume number.
const BAND_WIDTH: i32 = 50;
/// Minimum width of the draggable volume area.
const VOLUME_WIDTH: i32 = 260;
/// Approximate width (px) above which device/stream names are ellipsized.
const LABEL_MAX_WIDTH: i32 = 300;
/// How far (px) the mixer popover is shifted right of the button's right edge.
const POPOVER_OFFSET: i32 = 128;
/// Minimum height of each half of a row (the grid rows are homogeneous).
const HALF_HEIGHT: i32 = 26;
/// Width of the line marking the current volume.
const MARKER_WIDTH: f64 = 3.0;
/// Volume change per scroll step.
const SCROLL_STEP: f64 = 0.01;

/// Key coordinates of a mixer row: a hexagon split in half horizontally.
/// The top half holds the name; the bottom half has a band on each side
/// (icon/mute left, volume number right) around the volume area, which is
/// filled up to the current volume:
///
/// ```text
///    ______________________________
///   /  name                        \
///  <-------------------------------->
///   \ icon \██████████|      / 100 /
///    ‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾
/// ```
struct Shape {
    left: f64,
    right: f64,
    top: f64,
    mid: f64,
    bottom: f64,
    /// x where the left band's separator meets the middle line.
    inner_left: f64,
    /// x where the right band's separator meets the middle line.
    inner_right: f64,
}

impl Shape {
    fn new(width: i32, height: i32) -> Self {
        let half = LINE_WIDTH / 2.0;
        // The grid is inset by `ROW_TIP / 2 + LINE_WIDTH` on both sides (see
        // `Row::new`), so at three-quarter height, where the band content is
        // centered, each band is exactly as wide as its column.
        let band = LINE_WIDTH + BAND_WIDTH as f64;
        Self {
            left: half,
            right: width as f64 - half,
            top: half,
            mid: height as f64 / 2.0,
            bottom: height as f64 - half,
            inner_left: band,
            inner_right: width as f64 - band,
        }
    }

    /// x of the volume marker: the left separator is silent, the right one
    /// is `MAX_VOLUME`.
    fn volume_x(&self, volume: f64) -> f64 {
        let fraction = (volume / MAX_VOLUME).clamp(0.0, 1.0);
        self.inner_left + fraction * (self.inner_right - self.inner_left)
    }

    fn volume_at(&self, x: f64) -> f64 {
        let fraction =
            ((x - self.inner_left) / (self.inner_right - self.inner_left)).clamp(0.0, 1.0);
        // Same 1% steps as before.
        (fraction * MAX_VOLUME * 100.0).round() / 100.0
    }

    fn in_volume_area(&self, x: f64, y: f64) -> bool {
        y >= self.mid && (self.inner_left..=self.inner_right).contains(&x)
    }

    fn volume_area_path(&self, cr: &gtk::cairo::Context) {
        cr.move_to(self.inner_left, self.mid);
        cr.line_to(self.inner_right, self.mid);
        cr.line_to(self.inner_right - ROW_TIP, self.bottom);
        cr.line_to(self.inner_left + ROW_TIP, self.bottom);
        cr.close_path();
    }
}

/// Darkens the part of the fill above 100%, up to `volume`.
fn draw_overamplified(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, volume: f64) {
    if volume <= 1.0 {
        return;
    }
    let shape = Shape::new(area.width(), area.height());
    shape.volume_area_path(cr);
    cr.clip();
    let start = shape.volume_x(1.0);
    let end = shape.volume_x(volume);
    cr.rectangle(start, 0.0, end - start, area.height() as f64);
    set_source(cr, area);
    let _ = cr.fill();
}

/// Fills the volume area up to `volume`.
fn draw_volume_fill(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, volume: f64) {
    let shape = Shape::new(area.width(), area.height());
    shape.volume_area_path(cr);
    cr.clip();
    cr.rectangle(0.0, 0.0, shape.volume_x(volume), area.height() as f64);
    set_source(cr, area);
    let _ = cr.fill();
}

/// Draws the row outline and the marker at `volume`.
fn draw_row_outline(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, volume: f64) {
    let s = Shape::new(area.width(), area.height());

    // Outer hexagon.
    cr.move_to(s.left, s.mid);
    cr.line_to(s.left + ROW_TIP, s.top);
    cr.line_to(s.right - ROW_TIP, s.top);
    cr.line_to(s.right, s.mid);
    cr.line_to(s.right - ROW_TIP, s.bottom);
    cr.line_to(s.left + ROW_TIP, s.bottom);
    cr.close_path();
    // Middle line.
    cr.move_to(s.left, s.mid);
    cr.line_to(s.right, s.mid);
    // Separators between the bands and the volume area, parallel to the
    // outer edges next to them.
    cr.move_to(s.inner_left, s.mid);
    cr.line_to(s.inner_left + ROW_TIP, s.bottom);
    cr.move_to(s.inner_right, s.mid);
    cr.line_to(s.inner_right - ROW_TIP, s.bottom);
    cr.set_line_width(LINE_WIDTH);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    set_source(cr, area);
    let _ = cr.stroke();

    // Volume marker and a short tick at 100%, kept inside the volume area.
    s.volume_area_path(cr);
    cr.clip();
    let tick = s.volume_x(1.0);
    cr.move_to(tick, s.bottom);
    cr.line_to(tick, s.bottom - 4.0);
    let _ = cr.stroke();
    let marker = s.volume_x(volume);
    cr.move_to(marker, s.mid);
    cr.line_to(marker, s.bottom);
    cr.set_line_width(MARKER_WIDTH);
    let _ = cr.stroke();
}

fn format_volume(volume: f64) -> String {
    format!("{:.0}", volume * 100.0)
}

struct Row {
    root: gtk::Overlay,
    label: gtk::Label,
    icon: gtk::Image,
    volume: gtk::Label,
    fill: gtk::DrawingArea,
    overamplified: gtk::DrawingArea,
    outline: gtk::DrawingArea,
    /// Volume currently drawn; follows the endpoint except while dragging.
    level: Rc<Cell<f64>>,
    dragging: Rc<Cell<bool>>,
    mute: Rc<Cell<bool>>,
}

impl Row {
    fn new(id: u32) -> Self {
        let mute = Rc::new(Cell::new(false));
        // Impossible volume, so the first update always applies.
        let level = Rc::new(Cell::new(-1.0));
        let dragging = Rc::new(Cell::new(false));

        let label = gtk::Label::builder()
            .xalign(0.5)
            .ellipsize(pango::EllipsizeMode::End)
            .hexpand(true)
            .has_tooltip(true)
            .build();
        // GTK can only cap a label's width in characters, so convert
        // `LABEL_MAX_WIDTH` using the label's font.
        let char_width = label
            .pango_context()
            .metrics(None, None)
            .approximate_char_width()
            / pango::SCALE;
        label.set_max_width_chars(LABEL_MAX_WIDTH / char_width.max(1));
        // Show the full name, but only when it doesn't fit.
        label.connect_query_tooltip(|label, _, _, _, tooltip| {
            if !label.layout().is_ellipsized() {
                return false;
            }
            tooltip.set_text(Some(&label.text()));
            true
        });

        let icon = gtk::Image::new();
        let mute_button = gtk::Button::builder()
            .css_classes(["mixer-mute"])
            .child(&icon)
            .build();
        let muted = mute.clone();
        mute_button.connect_clicked(move |_| audio::get().set_mute(id, !muted.get()));

        let volume = gtk::Label::builder().css_classes(["mixer-volume"]).build();
        // Reserves the volume area's space; it is drawn by `fill`/`outline`.
        let volume_area = gtk::Box::builder()
            .hexpand(true)
            .width_request(VOLUME_WIDTH)
            .height_request(HALF_HEIGHT)
            .build();

        // [name                      ]
        // [mute | volume area | vol  ]
        // Equal row heights keep the drawn middle line between the halves.
        // The inset puts the band columns where `Shape` expects them.
        let inset = (ROW_TIP / 2.0 + LINE_WIDTH) as i32;
        let grid = gtk::Grid::builder()
            .row_homogeneous(true)
            .margin_start(inset)
            .margin_end(inset)
            .margin_top(2)
            .margin_bottom(2)
            .build();
        for widget in [mute_button.upcast_ref::<gtk::Widget>(), volume.upcast_ref()] {
            widget.set_width_request(BAND_WIDTH);
        }
        // Text reaches above quarter height, where the slanted outer edges
        // are further in; keep clear of them.
        label.set_margin_start((ROW_TIP / 4.0) as i32);
        label.set_margin_end((ROW_TIP / 4.0) as i32);
        grid.attach(&label, 0, 0, 3, 1);
        grid.attach(&mute_button, 0, 1, 1, 1);
        grid.attach(&volume_area, 1, 1, 1, 1);
        grid.attach(&volume, 2, 1, 1, 1);

        let fill = gtk::DrawingArea::builder()
            .css_classes(["mixer-volume-fill"])
            .build();
        let l = level.clone();
        fill.set_draw_func(move |area, cr, _, _| draw_volume_fill(area, cr, l.get()));
        // Over the fill, darkening the part above 100%.
        let overamplified = gtk::DrawingArea::builder()
            .css_classes(["mixer-overamplified"])
            .can_target(false)
            .build();
        let l = level.clone();
        overamplified.set_draw_func(move |area, cr, _, _| draw_overamplified(area, cr, l.get()));
        let outline = gtk::DrawingArea::builder()
            .css_classes(["mixer-row-outline"])
            .can_target(false)
            .build();
        let l = level.clone();
        outline.set_draw_func(move |area, cr, _, _| draw_row_outline(area, cr, l.get()));

        let root = gtk::Overlay::new();
        root.set_child(Some(&fill));
        root.add_overlay(&overamplified);
        root.add_overlay(&outline);
        root.add_overlay(&grid);
        root.set_measure_overlay(&grid, true);

        // Shows `value` right away and sends it to PipeWire; unmutes when
        // raised from zero and mutes at zero, like the old scale did.
        let set_volume: Rc<dyn Fn(f64)> = {
            let (level, volume, muted) = (level.clone(), volume.clone(), mute.clone());
            let layers = [fill.clone(), overamplified.clone(), outline.clone()];
            Rc::new(move |value: f64| {
                let value = value.clamp(0.0, MAX_VOLUME);
                level.set(value);
                volume.set_label(&format_volume(value));
                layers.iter().for_each(|layer| layer.queue_draw());
                let audio = audio::get();
                audio.set_volume(id, value);
                if value <= 0.0 && !muted.get() {
                    audio.set_mute(id, true);
                } else if value > 0.0 && muted.get() {
                    audio.set_mute(id, false);
                }
            })
        };

        // Press or drag anywhere in the volume area. Coordinates are
        // relative to `root`, which `fill`/`outline` fill exactly.
        let drag = gtk::GestureDrag::new();
        let (r, d, set) = (root.downgrade(), dragging.clone(), set_volume.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(root) = r.upgrade() else { return };
            let shape = Shape::new(root.width(), root.height());
            if !shape.in_volume_area(x, y) {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            d.set(true);
            set(shape.volume_at(x));
        });
        let (r, d, set) = (root.downgrade(), dragging.clone(), set_volume.clone());
        drag.connect_drag_update(move |gesture, dx, _| {
            let (Some(root), Some((x, _))) = (r.upgrade(), gesture.start_point()) else {
                return;
            };
            if d.get() {
                set(Shape::new(root.width(), root.height()).volume_at(x + dx));
            }
        });
        let d = dragging.clone();
        drag.connect_drag_end(move |_, _, _| d.set(false));
        root.add_controller(drag);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let (l, set) = (level.clone(), set_volume);
        scroll.connect_scroll(move |_, _, dy| {
            set(((l.get() - dy * SCROLL_STEP) * 100.0).round() / 100.0);
            glib::Propagation::Stop
        });
        root.add_controller(scroll);

        Self {
            root,
            label,
            icon,
            volume,
            fill,
            overamplified,
            outline,
            level,
            dragging,
            mute,
        }
    }

    fn update(&self, endpoint: &Endpoint, visible: bool) {
        self.root.set_visible(visible);
        self.label.set_label(&endpoint.description);
        self.icon.set_icon_name(Some(endpoint.volume_icon()));
        // Avoid fighting the user while they drag.
        if !self.dragging.get() && (self.level.get() - endpoint.volume).abs() > 0.005 {
            self.level.set(endpoint.volume);
            self.volume.set_label(&format_volume(endpoint.volume));
            self.fill.queue_draw();
            self.overamplified.queue_draw();
            self.outline.queue_draw();
        }
        self.mute.set(endpoint.mute);
        if endpoint.mute {
            self.fill.add_css_class("muted");
        } else {
            self.fill.remove_css_class("muted");
        }
    }
}

/// A list of endpoints of one kind, with the default one first.
struct Section {
    kind: Kind,
    container: gtk::Box,
    rows: RefCell<HashMap<u32, Row>>,
}

impl Section {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            container: gtk::Box::new(gtk::Orientation::Vertical, 4),
            rows: RefCell::default(),
        }
    }

    fn render(&self, state: &State, expanded: bool) {
        let default_id = state.default_endpoint(self.kind).map(|e| e.id);
        let mut endpoints: Vec<&Endpoint> = state.of_kind(self.kind).collect();
        endpoints.sort_by_key(|e| Some(e.id) != default_id);

        let mut rows = self.rows.borrow_mut();
        rows.retain(|id, row| {
            let keep = endpoints.iter().any(|e| e.id == *id);
            if !keep {
                self.container.remove(&row.root);
            }
            keep
        });

        let mut previous: Option<gtk::Widget> = None;
        for endpoint in endpoints {
            let row = rows.entry(endpoint.id).or_insert_with(|| {
                let row = Row::new(endpoint.id);
                self.container.append(&row.root);
                row
            });
            let visible = expanded || self.kind == Kind::Stream || Some(endpoint.id) == default_id;
            row.update(endpoint, visible);
            self.container
                .reorder_child_after(&row.root, previous.as_ref());
            previous = Some(row.root.clone().upcast());
        }
    }
}

fn popover_content() -> gtk::Box {
    let speakers = Section::new(Kind::Speaker);
    let microphones = Section::new(Kind::Microphone);
    let streams = Section::new(Kind::Stream);

    let streams_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    streams_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    streams_box.append(&streams.container);

    // Expand/collapse: a chevron on a small centered hexagon. The chevron
    // icons are bundled with the panel (see `resources/icons`).
    let toggle = gtk::Button::builder()
        .icon_name("vo1ded-chevron-down-symbolic")
        .css_classes(["mixer-toggle"])
        .tooltip_text("Show all devices and streams")
        .build();
    toggle.connect_clicked(|_| {
        let expanded = expanded();
        let value = *expanded.get();
        expanded.set(!value);
    });
    let toggle_hexagon = hexagon(&toggle, "mixer-toggle-background");
    toggle_hexagon.set_halign(gtk::Align::Center);
    toggle_hexagon.set_margin_top(4);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&speakers.container);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&microphones.container);
    content.append(&streams_box);
    content.append(&toggle_hexagon);

    let weak = content.downgrade();
    let render = Rc::new(move || {
        if weak.upgrade().is_none() {
            return false;
        }
        let audio = audio::get();
        let state = audio.state.get();
        let expanded = *expanded().get();
        speakers.render(&state, expanded);
        microphones.render(&state, expanded);
        streams.render(&state, expanded);
        streams_box.set_visible(expanded);
        if expanded {
            toggle.set_icon_name("vo1ded-chevron-up-symbolic");
            toggle.set_tooltip_text(Some("Show only default devices"));
        } else {
            toggle.set_icon_name("vo1ded-chevron-down-symbolic");
            toggle.set_tooltip_text(Some("Show all devices and streams"));
        }
        true
    });

    let r = render.clone();
    audio::get().state.subscribe(move |_| r());
    expanded().subscribe(move |_| render());

    content
}

/// The mixer button, on a hexagonal background.
pub fn wireplumber() -> gtk::Overlay {
    let icon = gtk::Image::new();
    let weak = icon.downgrade();
    audio::get().state.subscribe(move |state| {
        let Some(icon) = weak.upgrade() else {
            return false;
        };
        let name = state
            .default_endpoint(Kind::Speaker)
            .map_or(audio::ICON_SPEAKER_MUTED, Endpoint::volume_icon);
        icon.set_icon_name(Some(name));
        true
    });

    let popover = gtk::Popover::new();
    popover.set_child(Some(&popover_content()));
    // Open leftwards from the button's right edge. Centered, the popover
    // overflows the screen edge and the compositor slides it back, but that
    // adjustment is lost when it is resized (e.g. on Expand), making it jump.
    popover.set_halign(gtk::Align::End);
    // Shift it right so the arrow isn't squeezed into the corner; there is
    // room above the clock.
    popover.set_offset(POPOVER_OFFSET, 0);

    let button = gtk::MenuButton::new();
    button.add_css_class("wireplumber-button");
    button.set_child(Some(&icon));
    button.set_popover(Some(&popover));

    hexagon(&button, "wireplumber-background")
}
