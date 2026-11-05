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

    let center = gtk::Box::builder()
        .hexpand(true)
        .halign(gtk::Align::Center)
        .build();
    center.append(&widgets::workspaces::workspaces());

    let end = gtk::Box::builder()
        .hexpand(true)
        .halign(gtk::Align::End)
        .spacing(8)
        .build();
    end.append(&widgets::tray::tray());
    end.append(&widgets::wireplumber::wireplumber());
    end.append(&widgets::time::time());

    // GtkCenterBox's CSS node is called "box" by default; the stylesheet
    // targets `window.bar > centerbox`.
    let centerbox = gtk::glib::Object::builder::<gtk::CenterBox>()
        .property("css-name", "centerbox")
        .build();
    centerbox.set_center_widget(Some(&center));
    centerbox.set_end_widget(Some(&end));

    window.set_child(Some(&centerbox));
    window.present();
    window
}
