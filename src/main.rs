mod bar;
mod services;
mod steam_icon;
mod store;
mod widgets;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "dev.vo1ded.panel";
const STYLE: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

fn main() -> glib::ExitCode {
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::default());
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &gtk::Application) {
    // `activate` runs again if a second instance is launched; only set up once.
    if !app.windows().is_empty() {
        return;
    }

    let display = gdk::Display::default().expect("no display");

    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    // One bar per monitor, rebuilt when monitors are plugged or unplugged.
    let bars: Rc<RefCell<Vec<gtk::ApplicationWindow>>> = Rc::default();
    let rebuild = {
        let app = app.clone();
        let display = display.clone();
        let bars = bars.clone();
        move || {
            for bar in bars.borrow_mut().drain(..) {
                bar.destroy();
            }
            let monitors = display.monitors();
            for monitor in monitors.iter::<gdk::Monitor>().flatten() {
                bars.borrow_mut().push(bar::bar(&app, &monitor));
            }
        }
    };

    // Keep the application alive even while no monitors are connected.
    std::mem::forget(app.hold());

    rebuild();
    display
        .monitors()
        .connect_items_changed(move |_, _, _, _| rebuild());
}
