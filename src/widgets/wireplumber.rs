use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::services::audio::{self, Endpoint, Kind, State};
use crate::store::Store;

const MAX_VOLUME: f64 = 1.5;

thread_local! {
    /// Whether the mixer shows every device and stream, shared by all bars.
    static EXPANDED: Rc<Store<bool>> = Store::new(false);
}

fn expanded() -> Rc<Store<bool>> {
    EXPANDED.with(Rc::clone)
}

struct Row {
    root: gtk::Box,
    label: gtk::Label,
    icon: gtk::Image,
    scale: gtk::Scale,
    percent: gtk::Label,
    mute: Rc<Cell<bool>>,
}

impl Row {
    fn new(id: u32) -> Self {
        let mute = Rc::new(Cell::new(false));

        let label = gtk::Label::builder()
            .max_width_chars(1)
            .vexpand(true)
            .wrap(true)
            .build();

        let icon = gtk::Image::new();
        let mute_button = gtk::Button::builder()
            .css_classes(["btn-transparent"])
            .child(&icon)
            .build();
        let muted = mute.clone();
        mute_button.connect_clicked(move |_| audio::get().set_mute(id, !muted.get()));

        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, MAX_VOLUME, 0.01);
        scale.set_css_classes(&["horizontal", "marks-before", "wireplumber-endpoint-slider"]);
        // Scrolling moves by the page increment, which defaults to 10x the step.
        scale.set_increments(0.01, 0.01);
        scale.set_hexpand(true);
        scale.set_width_request(260);
        scale.add_mark(1.0, gtk::PositionType::Left, Some(""));

        let muted = mute.clone();
        scale.connect_change_value(move |scale, _, value| {
            let value = value.clamp(0.0, MAX_VOLUME);
            scale.set_value(value);
            let audio = audio::get();
            audio.set_volume(id, value);
            if value <= 0.0 && !muted.get() {
                audio.set_mute(id, true);
            } else if value > 0.0 && muted.get() {
                audio.set_mute(id, false);
            }
            glib::Propagation::Stop
        });

        let scale_box = gtk::Box::builder().hexpand(true).build();
        scale_box.append(&scale);

        let percent = gtk::Label::builder().width_request(40).build();

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        controls.append(&mute_button);
        controls.append(&scale_box);
        controls.append(&percent);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&label);
        root.append(&controls);

        Self {
            root,
            label,
            icon,
            scale,
            percent,
            mute,
        }
    }

    fn update(&self, endpoint: &Endpoint, visible: bool) {
        self.root.set_visible(visible);
        self.label.set_label(&endpoint.description);
        self.icon.set_icon_name(Some(endpoint.volume_icon()));
        // Avoid fighting the user while they drag the slider.
        if (self.scale.value() - endpoint.volume).abs() > 0.005 {
            self.scale.set_value(endpoint.volume);
        }
        self.percent
            .set_label(&format!("{:.0}%", endpoint.volume * 100.0));
        self.mute.set(endpoint.mute);
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
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
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

    let toggle = gtk::Button::new();
    toggle.connect_clicked(|_| {
        let expanded = expanded();
        let value = *expanded.get();
        expanded.set(!value);
    });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&speakers.container);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&microphones.container);
    content.append(&streams_box);
    content.append(&toggle);

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
        toggle.set_label(if expanded { "Collapse" } else { "Expand" });
        true
    });

    let r = render.clone();
    audio::get().state.subscribe(move |_| r());
    expanded().subscribe(move |_| render());

    content
}

pub fn wireplumber() -> gtk::Box {
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

    let button = gtk::MenuButton::new();
    button.set_child(Some(&icon));
    button.set_popover(Some(&popover));

    let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    container.append(&button);
    container
}
