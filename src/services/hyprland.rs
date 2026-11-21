//! Hyprland IPC: state is fetched over `.socket.sock` and refreshed whenever
//! an event arrives on `.socket2.sock`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use serde::Deserialize;

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Stable identifier used to match clients and the focused workspace.
    pub key: String,
    /// `None` for special workspaces on Hyprland versions that omit it.
    pub id: Option<i32>,
    pub name: String,
    pub special: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub address: String,
    /// [`Workspace::key`] of the workspace the client is on.
    pub workspace: String,
    pub class: String,
    pub pid: i32,
}

/// Workspaces shown on one monitor, as [`Workspace::key`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Monitor {
    pub active_workspace: Option<String>,
    /// Special workspace open on the monitor, if any.
    pub special_workspace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// Special workspaces first, then regular ones by id.
    pub workspaces: Vec<Workspace>,
    pub clients: Vec<Client>,
    /// Keyed by connector name, e.g. `DP-1`.
    pub monitors: BTreeMap<String, Monitor>,
    pub focused_monitor: Option<String>,
}

impl State {
    /// Whether the workspace is shown on `monitor` (as its active workspace
    /// or open special workspace). Unknown or missing monitors fall back to
    /// the focused one.
    pub fn is_active(&self, key: &str, monitor: Option<&str>) -> bool {
        let monitor = monitor
            .and_then(|m| self.monitors.get(m))
            .or_else(|| self.monitors.get(self.focused_monitor.as_deref()?));
        monitor.is_some_and(|m| {
            m.active_workspace.as_deref() == Some(key)
                || m.special_workspace.as_deref() == Some(key)
        })
    }
}

pub struct Hyprland {
    pub state: Rc<Store<State>>,
}

thread_local! {
    static INSTANCE: Rc<Hyprland> = Hyprland::start();
}

pub fn get() -> Rc<Hyprland> {
    INSTANCE.with(Rc::clone)
}

fn socket_dir() -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(runtime).join("hypr").join(signature))
}

fn request(command: &str) -> std::io::Result<String> {
    let path = socket_dir()
        .ok_or_else(|| std::io::Error::other("Hyprland is not running"))?
        .join(".socket.sock");
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(command.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn request_json<T: for<'de> Deserialize<'de>>(command: &str) -> Option<T> {
    let response = request(&format!("j/{command}"))
        .inspect_err(|e| eprintln!("hyprland: {command}: {e}"))
        .ok()?;
    serde_json::from_str(&response)
        .inspect_err(|e| eprintln!("hyprland: {command}: {e}"))
        .ok()
}

/// A workspace as referenced in any IPC response. Newer Hyprland versions
/// identify workspaces by `address` and omit `id` for special workspaces.
#[derive(Deserialize)]
struct WorkspaceRef {
    #[serde(default)]
    address: String,
    id: Option<i32>,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    type_: String,
}

impl WorkspaceRef {
    fn key(&self) -> String {
        if !self.address.is_empty() {
            self.address.clone()
        } else if let Some(id) = self.id {
            id.to_string()
        } else {
            self.name.clone()
        }
    }

    /// Monitors report an empty reference when no special workspace is open.
    fn is_none(&self) -> bool {
        self.address.is_empty() && self.name.is_empty() && self.id.is_none_or(|id| id == 0)
    }

    fn is_special(&self) -> bool {
        self.type_ == "special"
            || self.name.starts_with("special:")
            || self.id.is_some_and(|id| id < 0)
    }
}

#[derive(Deserialize)]
struct ClientJson {
    address: String,
    workspace: WorkspaceRef,
    #[serde(default)]
    class: String,
    #[serde(default)]
    pid: i32,
    #[serde(default = "default_true")]
    mapped: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct MonitorJson {
    name: String,
    focused: bool,
    #[serde(rename = "activeWorkspace")]
    active_workspace: Option<WorkspaceRef>,
    #[serde(rename = "specialWorkspace")]
    special_workspace: Option<WorkspaceRef>,
}

fn fetch() -> Option<State> {
    let mut workspaces: Vec<WorkspaceRef> = request_json("workspaces")?;
    workspaces
        .sort_by(|a, b| (!a.is_special(), a.id, &a.name).cmp(&(!b.is_special(), b.id, &b.name)));
    let clients: Vec<ClientJson> = request_json("clients")?;
    let monitors: Vec<MonitorJson> = request_json("monitors").unwrap_or_default();

    let focused_monitor = monitors.iter().find(|m| m.focused).map(|m| m.name.clone());
    let key = |w: Option<WorkspaceRef>| w.filter(|w| !w.is_none()).map(|w| w.key());
    let monitors = monitors
        .into_iter()
        .map(|m| {
            let monitor = Monitor {
                active_workspace: key(m.active_workspace),
                special_workspace: key(m.special_workspace),
            };
            (m.name, monitor)
        })
        .collect();

    Some(State {
        workspaces: workspaces
            .into_iter()
            .map(|w| Workspace {
                key: w.key(),
                id: w.id,
                special: w.is_special(),
                name: w.name,
            })
            .collect(),
        clients: clients
            .into_iter()
            .filter(|c| c.mapped)
            .map(|c| Client {
                address: c.address,
                workspace: c.workspace.key(),
                class: c.class,
                pid: c.pid,
            })
            .collect(),
        monitors,
        focused_monitor,
    })
}

/// Events that never change anything the bar shows; skipped to avoid
/// refetching on every window title change.
fn is_ignored(event: &str) -> bool {
    const IGNORED: &[&str] = &[
        "activewindow",
        "activewindowv2",
        "windowtitle",
        "windowtitlev2",
        "activelayout",
        "submap",
        "screencast",
        "bell",
    ];
    IGNORED.contains(&event)
}

fn listen(tx: async_channel::Sender<()>) {
    loop {
        let Some(path) = socket_dir().map(|d| d.join(".socket2.sock")) else {
            eprintln!("hyprland: HYPRLAND_INSTANCE_SIGNATURE not set, workspaces disabled");
            return;
        };
        match UnixStream::connect(&path) {
            Ok(stream) => {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let event = line.split(">>").next().unwrap_or_default();
                    if !is_ignored(event) && tx.send_blocking(()).is_err() {
                        return;
                    }
                }
            }
            Err(e) => eprintln!("hyprland: {}: {e}", path.display()),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

impl Hyprland {
    fn start() -> Rc<Self> {
        let this = Rc::new(Self {
            state: Store::new(fetch().unwrap_or_default()),
        });

        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || listen(tx));

        let state = this.state.clone();
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                // Collapse bursts of events into a single refresh.
                while rx.try_recv().is_ok() {}
                if let Some(new) = fetch() {
                    state.set(new);
                }
            }
        });

        this
    }

    /// Runs a Lua dispatcher expression, e.g. `hl.dsp.focus({ workspace = 3 })`.
    /// Hyprland evaluates it as `hl.dispatch(<dispatcher>)`.
    pub fn dispatch(&self, dispatcher: &str) {
        match request(&format!("dispatch {dispatcher}")) {
            Ok(reply) if reply.trim() == "ok" => {}
            // Lua errors come back as an `error: ...` reply, not an I/O error.
            Ok(reply) => eprintln!("hyprland: dispatch {dispatcher}: {}", reply.trim()),
            Err(e) => eprintln!("hyprland: dispatch {dispatcher}: {e}"),
        }
    }
}

/// Quotes `s` as a Lua string literal, for passing names to dispatchers.
pub fn lua_string(s: &str) -> String {
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            // Decimal escapes work in every Lua version, unlike `\u{..}`.
            c if c.is_ascii_control() => quoted.push_str(&format!("\\{:03}", c as u32)),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}
