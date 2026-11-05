use gtk::glib;
use gtk::prelude::*;

const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

fn now() -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format(FORMAT))
        .map(Into::into)
        .unwrap_or_default()
}

pub fn time() -> gtk::MenuButton {
    let label = gtk::Label::new(Some(&now()));

    let weak = label.downgrade();
    glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
        Some(label) => {
            label.set_label(&now());
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    let popover = gtk::Popover::new();
    popover.set_child(Some(&gtk::Calendar::new()));

    let button = gtk::MenuButton::new();
    button.add_css_class("transparent");
    button.set_child(Some(&label));
    button.set_popover(Some(&popover));
    button
}
