//! MPRIS media players.
//!
//! Players are discovered on the session bus through `NameOwnerChanged` and
//! tracked with `gio::DBusProxy`, which caches their properties and keeps
//! them up to date. Everything runs on the GTK main loop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;

use crate::store::Store;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// Well-known bus name, e.g. `org.mpris.MediaPlayer2.spotify`.
    pub name: String,
    pub status: Status,
    pub title: String,
    pub artists: Vec<String>,
    pub art_url: Option<String>,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

impl Player {
    /// Player name derived from the bus name, e.g. `spotify` or `firefox`.
    pub fn short_name(&self) -> &str {
        let name = self.name.strip_prefix(PREFIX).unwrap_or(&self.name);
        // Drop instance suffixes such as `firefox.instance_1_42`.
        name.split('.').next().unwrap_or(name)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// Players, most recently started playing first.
    pub players: Vec<Player>,
}

impl State {
    /// The player to show: the most recent one that is playing, otherwise the
    /// most recent paused one. Stopped players are not shown.
    pub fn active(&self) -> Option<&Player> {
        let with = |status| self.players.iter().find(|p| p.status == status);
        with(Status::Playing).or_else(|| with(Status::Paused))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
}

pub struct Mpris {
    pub state: Rc<Store<State>>,
    connection: RefCell<Option<gio::DBusConnection>>,
    proxies: RefCell<HashMap<String, gio::DBusProxy>>,
    /// Bus names whose cached properties changed. Proxy signal handlers must
    /// be `Send`, so they only forward the name to the main loop.
    changed: async_channel::Sender<String>,
    _names: RefCell<Option<gio::SignalSubscription>>,
}

thread_local! {
    static INSTANCE: Rc<Mpris> = Mpris::start();
}

pub fn get() -> Rc<Mpris> {
    INSTANCE.with(Rc::clone)
}

impl Mpris {
    fn start() -> Rc<Self> {
        let (changed_tx, changed_rx) = async_channel::unbounded::<String>();
        let this = Rc::new(Self {
            state: Store::new(State::default()),
            connection: RefCell::default(),
            proxies: RefCell::default(),
            changed: changed_tx,
            _names: RefCell::default(),
        });

        let mpris = this.clone();
        glib::spawn_future_local(async move {
            while let Ok(name) = changed_rx.recv().await {
                mpris.refresh(&name);
            }
        });

        let mpris = this.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = mpris.connect().await {
                eprintln!("mpris: {e}");
            }
        });

        this
    }

    async fn connect(self: &Rc<Self>) -> Result<(), glib::Error> {
        let connection = gio::bus_get_future(gio::BusType::Session).await?;
        self.connection.replace(Some(connection.clone()));

        // Subscribe before listing so no player can slip through in between.
        let weak = Rc::downgrade(self);
        let subscription = connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some("org.mpris.MediaPlayer2"),
            gio::DBusSignalFlags::MATCH_ARG0_NAMESPACE,
            move |signal| {
                let (Some(this), Some((name, old, new))) = (
                    weak.upgrade(),
                    signal.parameters.get::<(String, String, String)>(),
                ) else {
                    return;
                };
                if !old.is_empty() {
                    this.remove(&name);
                }
                if !new.is_empty() {
                    this.add(name);
                }
            },
        );
        self._names.replace(Some(subscription));

        let reply = connection
            .call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ListNames",
                None,
                Some(glib::VariantTy::new("(as)").unwrap()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        if let Some((names,)) = reply.get::<(Vec<String>,)>() {
            for name in names.into_iter().filter(|n| n.starts_with(PREFIX)) {
                self.add(name);
            }
        }
        Ok(())
    }

    fn add(self: &Rc<Self>, name: String) {
        if !name.starts_with(PREFIX) || self.proxies.borrow().contains_key(&name) {
            return;
        }
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let proxy = match gio::DBusProxy::new_future(
                &connection,
                // Some players invalidate properties instead of sending their
                // new values; have the proxy fetch them.
                gio::DBusProxyFlags::GET_INVALIDATED_PROPERTIES,
                None,
                Some(&name),
                PATH,
                INTERFACE,
            )
            .await
            {
                Ok(proxy) => proxy,
                Err(e) => {
                    eprintln!("mpris: {name}: {e}");
                    return;
                }
            };
            // The player may have quit while the proxy was being created.
            if proxy.name_owner().is_none() || this.proxies.borrow().contains_key(&name) {
                return;
            }
            let changed = this.changed.clone();
            let changed_name = name.clone();
            proxy.connect_g_properties_changed(move |_, _, _| {
                let _ = changed.send_blocking(changed_name.clone());
            });
            this.proxies.borrow_mut().insert(name.clone(), proxy);
            this.refresh(&name);
        });
    }

    fn remove(&self, name: &str) {
        if self.proxies.borrow_mut().remove(name).is_some() {
            self.state.update(|s| s.players.retain(|p| p.name != name));
        }
    }

    fn refresh(&self, name: &str) {
        let Some(player) = self.proxies.borrow().get(name).map(|p| read(name, p)) else {
            return;
        };
        let state = self.state.get();
        let index = state.players.iter().position(|p| p.name == name);
        if index.is_some_and(|i| state.players[i] == player) {
            return;
        }
        drop(state);

        self.state.update(|s| {
            let started = player.status == Status::Playing;
            match index {
                Some(i) if started && s.players[i].status != Status::Playing => {
                    s.players.remove(i);
                    s.players.insert(0, player);
                }
                Some(i) => s.players[i] = player,
                None if started => s.players.insert(0, player),
                None => s.players.push(player),
            }
        });
    }

    pub fn action(&self, name: &str, action: Action) {
        let Some(proxy) = self.proxies.borrow().get(name).cloned() else {
            return;
        };
        let method = match action {
            Action::PlayPause => "PlayPause",
            Action::Next => "Next",
            Action::Previous => "Previous",
        };
        proxy.call(
            method,
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
            |result| {
                if let Err(e) = result {
                    eprintln!("mpris: {e}");
                }
            },
        );
    }
}

fn read(name: &str, proxy: &gio::DBusProxy) -> Player {
    let flag = |key| {
        proxy
            .cached_property(key)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    let status = match proxy
        .cached_property("PlaybackStatus")
        .and_then(|v| v.get::<String>())
        .as_deref()
    {
        Some("Playing") => Status::Playing,
        Some("Paused") => Status::Paused,
        _ => Status::Stopped,
    };

    let metadata = proxy
        .cached_property("Metadata")
        .map(|m| glib::VariantDict::new(Some(&m)));
    let entry = |key| metadata.as_ref()?.lookup_value(key, None);
    let string = |key| entry(key)?.get::<String>().filter(|s| !s.is_empty());
    // `xesam:artist` should be a list, but some players send a plain string.
    let artists = entry("xesam:artist")
        .and_then(|v| {
            v.get::<Vec<String>>()
                .or_else(|| v.get::<String>().map(|s| vec![s]))
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.is_empty())
        .collect();

    Player {
        name: name.to_owned(),
        status,
        title: string("xesam:title").unwrap_or_default(),
        artists,
        art_url: string("mpris:artUrl"),
        can_play: flag("CanPlay"),
        can_pause: flag("CanPause"),
        can_go_next: flag("CanGoNext"),
        can_go_previous: flag("CanGoPrevious"),
    }
}
