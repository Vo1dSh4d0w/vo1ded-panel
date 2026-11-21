use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gio, glib};
use regex::Regex;

use crate::services::hyprland::{self, Client, State, Workspace, lua_string};
use crate::steam_icon::try_grab_steam_icon;
use crate::widgets::hexagon::hex_row;

#[derive(Clone)]
enum Icon {
    App(gio::Icon),
    File(PathBuf),
    Name(String),
}

thread_local! {
    /// (desktop entry name without ".desktop", lowercased) -> icon
    static APPS: Vec<(String, Option<gio::Icon>)> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show())
        .filter_map(|app| {
            let id = app.id()?.to_lowercase();
            let entry = id.split(".desktop").next().unwrap_or_default().to_owned();
            Some((entry, app.icon()))
        })
        .collect();

    /// Icon lookups can hit the disk (Steam), so cache them per (class, pid).
    static ICON_CACHE: RefCell<HashMap<(String, i32), Icon>> = RefCell::default();
}

fn find_app_icon(class: &str) -> Option<gio::Icon> {
    if class.is_empty() {
        return None;
    }
    let class = class.to_lowercase();
    // The class is used as a pattern against the desktop entry name; fall
    // back to a plain substring match if it is not a valid regex.
    let matches: Box<dyn Fn(&str) -> bool> = match Regex::new(&class) {
        Ok(re) => Box::new(move |entry| re.is_match(entry)),
        Err(_) => Box::new(move |entry| entry.contains(&class)),
    };
    APPS.with(|apps| {
        apps.iter()
            .filter(|(entry, _)| matches(entry))
            .filter_map(|(_, icon)| icon.clone())
            .next_back()
    })
}

fn client_icon(client: &Client) -> Icon {
    let key = (client.class.clone(), client.pid);
    if let Some(icon) = ICON_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return icon;
    }

    let icon = if let Some(icon) = find_app_icon(&client.class) {
        Icon::App(icon)
    } else if let Some(path) = try_grab_steam_icon(client.pid, &client.class) {
        Icon::File(path)
    } else {
        Icon::Name(client.class.clone())
    };

    ICON_CACHE.with(|c| c.borrow_mut().insert(key, icon.clone()));
    icon
}

fn icon_image(icon: Icon) -> gtk::Image {
    match icon {
        Icon::App(icon) => gtk::Image::from_gicon(&icon),
        Icon::File(path) => gtk::Image::from_file(path),
        Icon::Name(name) => gtk::Image::from_icon_name(&name),
    }
}

fn special_name(workspace: &Workspace) -> &str {
    workspace
        .name
        .split_once(':')
        .map_or(workspace.name.as_str(), |(_, name)| name)
}

fn workspace_label(workspace: &Workspace) -> String {
    if workspace.special {
        special_name(workspace)
            .chars()
            .next()
            .map_or_else(|| "S".to_owned(), |c| c.to_uppercase().collect())
    } else {
        workspace
            .id
            .map_or_else(|| workspace.name.clone(), |id| id.to_string())
    }
}

fn workspace_button(workspace: &Workspace, state: &State, active: bool) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&gtk::Label::new(Some(&workspace_label(workspace))));
    // Newest client first, matching the AGS version.
    for client in state
        .clients
        .iter()
        .rev()
        .filter(|c| c.workspace == workspace.key)
    {
        content.append(&icon_image(client_icon(client)));
    }

    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("workspace");
    button.add_css_class(if active {
        "workspace-active"
    } else {
        "workspace-inactive"
    });

    let dispatch = if workspace.special {
        format!(
            "hl.dsp.workspace.toggle_special({})",
            lua_string(special_name(workspace))
        )
    } else if let Some(id) = workspace.id {
        format!("hl.dsp.focus({{ workspace = {id} }})")
    } else {
        format!(
            "hl.dsp.focus({{ workspace = {} }})",
            lua_string(&format!("name:{}", workspace.name))
        )
    };
    button.connect_clicked(move |_| hyprland::get().dispatch(&dispatch));

    button
}

/// Workspaces, highlighting the ones shown on `monitor` (a connector name
/// such as `DP-1`; the focused monitor if `None` or unknown to Hyprland).
pub fn workspaces(monitor: Option<String>) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);

    let weak: glib::WeakRef<gtk::Box> = container.downgrade();
    hyprland::get().state.subscribe(move |state| {
        let Some(container) = weak.upgrade() else {
            return false;
        };
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        let is_active = |w: &Workspace| state.is_active(&w.key, monitor.as_deref());

        // Each special workspace is a standalone hexagon, left of the rest.
        for workspace in state.workspaces.iter().filter(|w| w.special) {
            let active = is_active(workspace);
            let button = workspace_button(workspace, state, active);
            container.append(&hex_row(&[button], Some(0), active, "workspace"));
        }

        let regular: Vec<&Workspace> = state.workspaces.iter().filter(|w| !w.special).collect();
        if !regular.is_empty() {
            let active = regular.iter().position(|w| is_active(w));
            let buttons: Vec<gtk::Button> = regular
                .iter()
                .enumerate()
                .map(|(i, w)| workspace_button(w, state, active == Some(i)))
                .collect();
            container.append(&hex_row(&buttons, active, true, "workspace"));
        }
        true
    });

    container
}
