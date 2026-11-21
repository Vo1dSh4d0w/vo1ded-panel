use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::services::mpris::{self, Action, Player, Status};
use crate::widgets::hexagon::hex_row;

const FALLBACK_ART: &str = "audio-x-generic-symbolic";
const ART_SIZE: i32 = 24;

/// Bus name of the player the widget is currently showing.
type Current = Rc<RefCell<Option<String>>>;

fn control(icon: &str, action: Action, current: &Current) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    let current = current.clone();
    button.connect_clicked(move |_| {
        if let Some(name) = current.borrow().as_deref() {
            mpris::get().action(name, action);
        }
    });
    button
}

fn label_text(player: &Player) -> String {
    let title = if player.title.is_empty() {
        player.short_name()
    } else {
        &player.title
    };
    if player.artists.is_empty() {
        title.to_owned()
    } else {
        format!("{} — {title}", player.artists.join(", "))
    }
}

/// Shows `url` in `image`, falling back to an icon while it loads or if it
/// can't be loaded. `shown` remembers the URL so unchanged art isn't reloaded.
fn set_art(image: &gtk::Image, url: Option<&str>, shown: &Rc<RefCell<Option<String>>>) {
    if shown.borrow().as_deref() == url {
        return;
    }
    shown.replace(url.map(str::to_owned));
    image.set_icon_name(Some(FALLBACK_ART));
    let Some(url) = url.map(str::to_owned) else {
        return;
    };

    let weak = image.downgrade();
    let shown = shown.clone();
    glib::spawn_future_local(async move {
        // `file://` always works; remote URLs need GVfs to be available.
        let Ok((bytes, _)) = gio::File::for_uri(&url).load_bytes_future().await else {
            return;
        };
        let Ok(texture) = gdk::Texture::from_bytes(&bytes) else {
            return;
        };
        // The track may have changed while loading.
        if shown.borrow().as_deref() == Some(url.as_str())
            && let Some(image) = weak.upgrade()
        {
            image.set_paintable(Some(&texture));
        }
    });
}

/// The currently playing media. Hidden while no player is playing or paused.
pub fn media() -> gtk::Box {
    let current: Current = Rc::default();
    let art_url = Rc::new(RefCell::new(None));

    let art = gtk::Image::builder()
        .pixel_size(ART_SIZE)
        .icon_name(FALLBACK_ART)
        .css_classes(["media-art"])
        .build();
    let label = gtk::Label::builder()
        .ellipsize(pango::EllipsizeMode::End)
        .max_width_chars(40)
        .build();
    let previous = control("media-skip-backward-symbolic", Action::Previous, &current);
    let play_pause = control("media-playback-start-symbolic", Action::PlayPause, &current);
    let next = control("media-skip-forward-symbolic", Action::Next, &current);
    play_pause.add_css_class("media-play-pause");
    previous.add_css_class("media-skip");
    next.add_css_class("media-skip");
    // Play/pause is the full, filled hexagon; previous/next point away from it.
    let controls = hex_row(
        &[previous.clone(), play_pause.clone(), next.clone()],
        Some(1),
        true,
        "media-controls",
    );

    let root = gtk::Box::builder()
        .spacing(6)
        .css_classes(["media"])
        .visible(false)
        .build();
    root.append(&art);
    root.append(&label);
    root.append(&controls);

    let weak = root.downgrade();
    mpris::get().state.subscribe(move |state| {
        let Some(root) = weak.upgrade() else {
            return false;
        };
        let player = state.active();
        root.set_visible(player.is_some());
        current.replace(player.map(|p| p.name.clone()));
        let Some(player) = player else {
            return true;
        };

        let text = label_text(player);
        label.set_label(&text);
        root.set_tooltip_text(Some(&text));
        set_art(&art, player.art_url.as_deref(), &art_url);

        let playing = player.status == Status::Playing;
        play_pause.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        play_pause.set_sensitive(if playing {
            player.can_pause
        } else {
            player.can_play
        });
        previous.set_sensitive(player.can_go_previous);
        next.set_sensitive(player.can_go_next);
        true
    });

    root
}
