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
    // GTK's GPU renderers intermittently clip the tops of glyphs in labels
    // (seen with GTK 4.22). The bar is cheap to draw, so default to the
    // software renderer; setting `GSK_RENDERER` explicitly still overrides it.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: nothing else is running yet; no threads have been spawned.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }

    let app_id = std::env::var("VO1DED_APP_ID").unwrap_or(APP_ID.to_string());

    gio::resources_register_include!("resources.gresource").expect("failed to register resources");

    let app = gtk::Application::new(Some(app_id), gio::ApplicationFlags::default());
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &gtk::Application) {
    // `activate` runs again if a second instance is launched; only set up once.
    if !app.windows().is_empty() {
        return;
    }

    let display = gdk::Display::default().expect("no display");

    // Bundled icons are looked up as part of hicolor (after the user's theme),
    // so they use `vo1ded-` prefixed names that no theme will shadow.
    gtk::IconTheme::for_display(&display).add_resource_path("/dev/vo1ded/panel/icons");

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
