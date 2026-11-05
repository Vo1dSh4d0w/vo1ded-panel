use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use system_tray::client::ActivateRequest;
use system_tray::item::{IconPixmap, StatusNotifierItem};
use system_tray::menu::{MenuItem, MenuType, ToggleState, ToggleType};

use crate::services::tray::{self, Command, Item};

/// Data the click handler needs, refreshed on every item update.
#[derive(Default)]
struct ClickInfo {
    address: String,
    menu_path: String,
    is_menu: bool,
}

struct TrayButton {
    button: gtk::MenuButton,
    image: gtk::Image,
    popover: gtk::PopoverMenu,
    info: Rc<RefCell<ClickInfo>>,
}

fn pixmap_texture(pixmaps: &[IconPixmap]) -> Option<gdk::MemoryTexture> {
    let pixmap = pixmaps
        .iter()
        .filter(|p| p.width > 0)
        .max_by_key(|p| p.width)?;
    let width = pixmap.width as usize;
    // The height reported by system-tray is unreliable; derive it instead.
    let height = pixmap.pixels.len() / 4 / width;
    if height == 0 {
        return None;
    }
    // ARGB32 in network byte order -> RGBA
    let rgba: Vec<u8> = pixmap.pixels[..width * height * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[1], p[2], p[3], p[0]])
        .collect();
    Some(gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(rgba),
        width * 4,
    ))
}

fn set_icon(image: &gtk::Image, item: &StatusNotifierItem) {
    if let Some(path) = item.icon_theme_path.as_deref().filter(|p| !p.is_empty())
        && let Some(display) = gdk::Display::default()
    {
        let theme = gtk::IconTheme::for_display(&display);
        if !theme.search_path().iter().any(|p| p.as_os_str() == path) {
            theme.add_search_path(path);
        }
    }

    match item.icon_name.as_deref().filter(|n| !n.is_empty()) {
        Some(path) if path.starts_with('/') => image.set_from_file(Some(path)),
        Some(name) => image.set_icon_name(Some(name)),
        None => match item.icon_pixmap.as_deref().and_then(pixmap_texture) {
            Some(texture) => image.set_paintable(Some(&texture)),
            None => image.set_icon_name(Some("image-missing")),
        },
    }
}

fn tooltip_markup(item: &StatusNotifierItem) -> Option<String> {
    match &item.tool_tip {
        Some(tip) if !tip.description.is_empty() => {
            Some(format!("{}\n{}", tip.title, tip.description))
        }
        Some(tip) if !tip.title.is_empty() => Some(tip.title.clone()),
        _ => item.title.clone(),
    }
}

fn build_menu(
    items: &[MenuItem],
    actions: &gio::SimpleActionGroup,
    address: &str,
    menu_path: &str,
) -> gio::Menu {
    let root = gio::Menu::new();
    let mut section = gio::Menu::new();

    for item in items.iter().filter(|i| i.visible) {
        if item.menu_type == MenuType::Separator {
            if section.n_items() > 0 {
                root.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }

        let label = item.label.as_deref().unwrap_or_default();

        if !item.submenu.is_empty() {
            let submenu = build_menu(&item.submenu, actions, address, menu_path);
            section.append_submenu(Some(label), &submenu);
            continue;
        }

        let name = format!("item-{}", item.id);
        let action = match item.toggle_type {
            ToggleType::CannotBeToggled => gio::SimpleAction::new(&name, None),
            ToggleType::Checkmark | ToggleType::Radio => gio::SimpleAction::new_stateful(
                &name,
                None,
                &(item.toggle_state == ToggleState::On).to_variant(),
            ),
        };
        action.set_enabled(item.enabled);
        let request = ActivateRequest::MenuItem {
            address: address.to_owned(),
            menu_path: menu_path.to_owned(),
            submenu_id: item.id,
        };
        action.connect_activate(move |_, _| {
            tray::get().send(Command::Activate(request.clone()));
        });
        actions.add_action(&action);

        let menu_item = gio::MenuItem::new(Some(label), Some(&format!("dbusmenu.{name}")));
        if let Some(icon) = item.icon_name.as_deref().filter(|n| !n.is_empty()) {
            menu_item.set_icon(&gio::ThemedIcon::new(icon));
        }
        section.append_item(&menu_item);
    }

    if section.n_items() > 0 {
        root.append_section(None, &section);
    }
    root
}

fn handle_event(button: &gtk::MenuButton, info: &ClickInfo, event: &gdk::Event) -> bool {
    let Some(event) = event.downcast_ref::<gdk::ButtonEvent>() else {
        return true;
    };
    let mouse_button = event.button();

    match event.event_type() {
        gdk::EventType::ButtonPress => {
            if mouse_button == gdk::BUTTON_SECONDARY
                || (info.is_menu && mouse_button == gdk::BUTTON_PRIMARY)
            {
                tray::get().send(Command::AboutToShow {
                    address: info.address.clone(),
                    menu_path: info.menu_path.clone(),
                });
            }
        }
        gdk::EventType::ButtonRelease => {
            // Releases inside the open popover belong to the menu.
            if Some(event.surface()) != button.native().map(|n| n.surface()) {
                return false;
            }
            if mouse_button == gdk::BUTTON_PRIMARY {
                if info.is_menu {
                    button.popup();
                } else {
                    let (x, y) = event.position().unwrap_or_default();
                    tray::get().send(Command::Activate(ActivateRequest::Default {
                        address: info.address.clone(),
                        x: x as i32,
                        y: y as i32,
                    }));
                }
            } else if mouse_button == gdk::BUTTON_SECONDARY {
                button.popup();
            }
        }
        _ => {}
    }
    true
}

impl TrayButton {
    fn new() -> Self {
        let image = gtk::Image::new();
        let popover =
            gtk::PopoverMenu::from_model_full(&gio::Menu::new(), gtk::PopoverMenuFlags::NESTED);
        let button = gtk::MenuButton::builder()
            .css_classes(["tray-item"])
            .child(&image)
            .popover(&popover)
            .build();
        let info: Rc<RefCell<ClickInfo>> = Rc::default();

        let controller = gtk::EventControllerLegacy::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = button.downgrade();
        let click_info = info.clone();
        controller.connect_event(move |_, event| {
            let stop = weak
                .upgrade()
                .is_some_and(|button| handle_event(&button, &click_info.borrow(), event));
            if stop {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        button.add_controller(controller);

        Self {
            button,
            image,
            popover,
            info,
        }
    }

    fn update(&self, item: &Item) {
        let menu_path = item.item.menu.clone().unwrap_or_default();
        *self.info.borrow_mut() = ClickInfo {
            address: item.address.clone(),
            menu_path: menu_path.clone(),
            is_menu: item.item.item_is_menu,
        };

        set_icon(&self.image, &item.item);
        self.button
            .set_tooltip_markup(tooltip_markup(&item.item).as_deref());

        let actions = gio::SimpleActionGroup::new();
        let menu = item
            .menu
            .as_ref()
            .map(|m| build_menu(&m.submenus, &actions, &item.address, &menu_path));
        self.button.insert_action_group("dbusmenu", Some(&actions));
        self.popover.set_menu_model(menu.as_ref());
    }
}

pub fn tray() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    container.add_css_class("tray");

    let buttons: RefCell<HashMap<String, TrayButton>> = RefCell::default();
    let weak = container.downgrade();
    tray::get().items.subscribe(move |items| {
        let Some(container) = weak.upgrade() else {
            return false;
        };
        let mut buttons = buttons.borrow_mut();

        // Only items that expose a menu are shown.
        let shown: Vec<&Item> = items
            .iter()
            .filter(|i| i.item.menu.is_some() && i.menu.is_some())
            .collect();

        buttons.retain(|address, button| {
            let keep = shown.iter().any(|i| &i.address == address);
            if !keep {
                container.remove(&button.button);
            }
            keep
        });

        let mut previous: Option<gtk::Widget> = None;
        for item in shown {
            let button = buttons.entry(item.address.clone()).or_insert_with(|| {
                let button = TrayButton::new();
                container.append(&button.button);
                button
            });
            button.update(item);
            container.reorder_child_after(&button.button, previous.as_ref());
            previous = Some(button.button.clone().upcast());
        }
        true
    });

    container
}
