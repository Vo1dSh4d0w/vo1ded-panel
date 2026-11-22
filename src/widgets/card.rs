//! A hexagonal card with an image, a title, a body, action buttons and a
//! dismiss button: the look shared by notifications and dialogs.
//!
//! ```text
//!    ____________________________ _
//!   / ___                       </>\
//!  < /img\  title                   >
//!   \\___/  body                   /
//!    ‾‾‾‾‾‾‾ <action> <action> ‾‾‾‾
//! ```
//!
//! Styled through the `notification-*` CSS classes (see
//! `styles/notifications.scss`).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::services::notifications::{Action, Image, Urgency};
use crate::widgets::hexagon::{LINE_WIDTH, TIP, hex_row, hexagon, set_source};

/// Width of a card.
const WIDTH: i32 = 400;
/// Horizontal size of a card's pointed sides. Cards vary in height, so the
/// angle varies; the width is fixed to keep content clear.
const CARD_TIP: f64 = 24.0;
/// Height of the hexagon holding the image; its width makes it regular.
const IMAGE_HEIGHT: i32 = 48;
/// Size of themed icons inside the image hexagon.
const ICON_SIZE: i32 = 32;
/// Lines of body text shown before it scrolls.
const BODY_LINES: i32 = 6;

/// What a card shows.
pub struct Card<'a> {
    /// Plain text.
    pub title: &'a str,
    /// Pango markup, already sanitized; empty for no body.
    pub body_markup: &'a str,
    pub image: Option<&'a Image>,
    pub actions: &'a [Action],
    /// Action keys are icon names, shown instead of the labels.
    pub action_icons: bool,
    pub urgency: Urgency,
}

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

/// The card's outline, inset by half the line width so it isn't clipped.
fn outline_path(area: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
    let half = LINE_WIDTH / 2.0;
    let rect = (
        half,
        half,
        area.width() as f64 - LINE_WIDTH,
        area.height() as f64 - LINE_WIDTH,
    );
    hexagon_path(cr, rect, CARD_TIP);
}

/// Loads an image as a texture; `None` for themed icons or unreadable files.
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
            Some(
                gdk::MemoryTexture::new(*width, *height, format, data, *rowstride as usize)
                    .upcast(),
            )
        }
        Image::File(path) => gdk::Texture::from_file(&gio::File::for_path(path))
            .inspect_err(|e| eprintln!("card: {path}: {e}"))
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

/// The image in a hexagon: pictures fill and are clipped to it, themed icons
/// sit on a filled hexagon.
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

/// Builds a card. `on_action` gets the key of a clicked action button,
/// `on_dismiss` runs when the dismiss button is clicked.
pub fn card(
    card: &Card,
    on_action: impl Fn(&str) + 'static,
    on_dismiss: impl Fn() + 'static,
) -> gtk::Overlay {
    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(2)
        .margin_start(CARD_TIP as i32)
        .margin_end(CARD_TIP as i32)
        .margin_top(8)
        .margin_bottom(8)
        .build();

    if let Some(image) = card.image.and_then(image_hexagon) {
        // A regular hexagon, whose tips match the bar's (`TIP`) at this
        // height.
        let width = (IMAGE_HEIGHT as f64 * 2.0 / 3f64.sqrt()).round() as i32;
        image.set_size_request(width, IMAGE_HEIGHT);
        image.set_valign(gtk::Align::Center);
        image.set_halign(gtk::Align::Start);
        grid.attach(&image, 0, 0, 1, 2);
    }

    let title = gtk::Label::builder()
        .label(card.title)
        .css_classes(["notification-summary"])
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        // Natural width comes from the card, not the text.
        .max_width_chars(1)
        // Clear of the dismiss button.
        .margin_end(32)
        .build();
    title.set_tooltip_text(Some(card.title));
    grid.attach(&title, 1, 0, 1, 1);

    if !card.body_markup.is_empty() {
        let body = gtk::Label::builder()
            .css_classes(["notification-body"])
            .use_markup(true)
            .label(card.body_markup)
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
            if let Err(e) =
                gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>)
            {
                eprintln!("card: {uri}: {e}");
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

    if !card.actions.is_empty() {
        let on_action = Rc::new(on_action);
        let actions = gtk::Box::builder()
            .spacing(4)
            .halign(gtk::Align::Center)
            .margin_top(4)
            .build();
        for action in card.actions {
            let button = if card.action_icons {
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
            let (on_action, key) = (on_action.clone(), action.key.clone());
            button.connect_clicked(move |_| on_action(&key));
            actions.append(&hex_row(&[button], Some(0), true, "notification-action"));
        }
        grid.attach(&actions, 0, 2, 2, 1);
    }

    let dismiss = gtk::Button::builder()
        .icon_name("vo1ded-window-close-symbolic")
        .css_classes(["notification-dismiss"])
        .tooltip_text("Dismiss")
        .build();
    dismiss.connect_clicked(move |_| on_dismiss());
    let dismiss = hexagon(&dismiss, "notification-dismiss-background");
    dismiss.set_halign(gtk::Align::End);
    dismiss.set_valign(gtk::Align::Start);
    // Its right tip stays inside the upper right slant.
    dismiss.set_margin_end(CARD_TIP as i32);
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
            match card.urgency {
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
    root
}
