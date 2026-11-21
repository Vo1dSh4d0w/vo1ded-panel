use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gio, glib};
use regex::Regex;

use crate::services::hyprland::{self, Client, State, Workspace};
use crate::steam_icon::try_grab_steam_icon;

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

fn workspace_button(workspace: &Workspace, state: &State) -> gtk::Button {
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

    // Keep the content clear of the chevrons drawn between buttons.
    content.set_margin_start(CONTENT_INSET);
    content.set_margin_end(CONTENT_INSET);

    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("workspace");
    button.add_css_class(if state.is_active(&workspace.key) {
        "workspace-active"
    } else {
        "workspace-inactive"
    });

    let dispatch = if workspace.special {
        format!("togglespecialworkspace {}", special_name(workspace))
    } else if let Some(id) = workspace.id {
        format!("workspace {id}")
    } else {
        format!("workspace name:{}", workspace.name)
    };
    button.connect_clicked(move |_| hyprland::get().dispatch(&dispatch));

    button
}

/// Horizontal size of a chevron tip.
const TIP: f64 = 14.0;
const LINE_WIDTH: f64 = 1.0;
/// Space between a button's edge and its content: just enough to clear the
/// chevron tip reaching into the button (half a tip plus half the line),
/// plus a 1px gap.
const CONTENT_INSET: i32 = (TIP / 2.0 + LINE_WIDTH / 2.0) as i32 + 1;

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

fn set_source(cr: &gtk::cairo::Context, area: &gtk::DrawingArea) {
    let color = area.color();
    cr.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        color.alpha().into(),
    );
}

/// A row of buttons outlined as interlocking hexagon halves; the `center`
/// button is a full hexagon, filled if `fill_center`. The fill and outline
/// are drawn in the CSS `color` of `.workspace-fill` / `.workspace-outline`.
fn hex_row(buttons: &[gtk::Button], center: Option<usize>, fill_center: bool) -> gtk::Overlay {
    let row = gtk::Box::builder()
        // Room for the outermost tips.
        .margin_start((TIP / 2.0 + LINE_WIDTH) as i32)
        .margin_end((TIP / 2.0 + LINE_WIDTH) as i32)
        .build();
    for button in buttons {
        row.append(button);
    }

    let fill = gtk::DrawingArea::builder()
        .css_classes(["workspace-fill"])
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
        .css_classes(["workspace-outline"])
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

pub fn workspaces() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);

    let weak: glib::WeakRef<gtk::Box> = container.downgrade();
    hyprland::get().state.subscribe(move |state| {
        let Some(container) = weak.upgrade() else {
            return false;
        };
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }

        // Each special workspace is a standalone hexagon, left of the rest.
        for workspace in state.workspaces.iter().filter(|w| w.special) {
            let button = workspace_button(workspace, state);
            let active = state.is_active(&workspace.key);
            container.append(&hex_row(&[button], Some(0), active));
        }

        let regular: Vec<&Workspace> = state.workspaces.iter().filter(|w| !w.special).collect();
        if !regular.is_empty() {
            let active = regular.iter().position(|w| state.is_active(&w.key));
            let buttons: Vec<gtk::Button> =
                regular.iter().map(|w| workspace_button(w, state)).collect();
            container.append(&hex_row(&buttons, active, true));
        }
        true
    });

    container
}
