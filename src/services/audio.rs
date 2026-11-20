//! Native PipeWire audio mixer.
//!
//! A dedicated thread runs the PipeWire main loop and tracks audio nodes,
//! their `Props` (volume/mute), device `Route`s and the `default` metadata,
//! mirroring what WirePlumber exposes. Snapshots are sent to the GTK main
//! loop; volume/mute changes are sent back as [`Command`]s.
//!
//! Volumes use the same cubic scale as `wpctl` and AstalWp: 1.0 is 100%.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use pipewire as pw;
use pw::device::{Device, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::registry::GlobalObject;
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pw::spa::utils::dict::DictRef;
use pw::types::ObjectType;

use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Speaker,
    Microphone,
    Stream,
}

impl Kind {
    fn from_media_class(class: &str) -> Option<Self> {
        match class {
            "Audio/Sink" => Some(Self::Speaker),
            "Audio/Source" => Some(Self::Microphone),
            "Stream/Output/Audio" => Some(Self::Stream),
            _ => None,
        }
    }
}

pub const ICON_SPEAKER_MUTED: &str = "vo1ded-audio-volume-muted-symbolic";
pub const ICON_MIC_MUTED: &str = "vo1ded-microphone-sensitivity-muted-symbolic";

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub id: u32,
    pub kind: Kind,
    /// `node.name`, used to match the default device.
    pub name: String,
    pub description: String,
    pub volume: f64,
    pub mute: bool,
}

impl Endpoint {
    /// Name of the bundled icon (see `resources/icons`) for this endpoint's
    /// level. Volumes above 100% use the "high" icon.
    pub fn volume_icon(&self) -> &'static str {
        let mic = self.kind == Kind::Microphone;
        if self.mute || self.volume <= 0.0 {
            if mic {
                ICON_MIC_MUTED
            } else {
                ICON_SPEAKER_MUTED
            }
        } else if self.volume <= 0.33 {
            if mic {
                "vo1ded-microphone-sensitivity-low-symbolic"
            } else {
                "vo1ded-audio-volume-low-symbolic"
            }
        } else if self.volume <= 0.66 {
            if mic {
                "vo1ded-microphone-sensitivity-medium-symbolic"
            } else {
                "vo1ded-audio-volume-medium-symbolic"
            }
        } else if mic {
            "vo1ded-microphone-sensitivity-high-symbolic"
        } else {
            "vo1ded-audio-volume-high-symbolic"
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub endpoints: BTreeMap<u32, Endpoint>,
    default_speaker: Option<String>,
    default_microphone: Option<String>,
}

impl State {
    pub fn default_endpoint(&self, kind: Kind) -> Option<&Endpoint> {
        let name = match kind {
            Kind::Speaker => self.default_speaker.as_ref()?,
            Kind::Microphone => self.default_microphone.as_ref()?,
            Kind::Stream => return None,
        };
        self.endpoints
            .values()
            .find(|e| e.kind == kind && &e.name == name)
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Endpoint> {
        self.endpoints.values().filter(move |e| e.kind == kind)
    }
}

pub enum Command {
    SetVolume(u32, f64),
    SetMute(u32, bool),
}

enum Message {
    Upsert(Endpoint),
    Remove(u32),
    DefaultSpeaker(Option<String>),
    DefaultMicrophone(Option<String>),
    Reset,
}

pub struct Audio {
    pub state: Rc<Store<State>>,
    commands: pw::channel::Sender<Command>,
}

thread_local! {
    static INSTANCE: Rc<Audio> = Audio::start();
}

pub fn get() -> Rc<Audio> {
    INSTANCE.with(Rc::clone)
}

impl Audio {
    fn start() -> Rc<Self> {
        let (msg_tx, msg_rx) = async_channel::unbounded();
        let (cmd_tx, cmd_rx) = pw::channel::channel();

        std::thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || {
                pw::init();
                let mut commands = cmd_rx;
                loop {
                    commands = match run(&msg_tx, commands) {
                        Ok(commands) => commands,
                        Err(e) => {
                            eprintln!("pipewire: {e}");
                            return;
                        }
                    };
                    let _ = msg_tx.send_blocking(Message::Reset);
                    // Lost the connection; wait for PipeWire to come back.
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("failed to spawn pipewire thread");

        let this = Rc::new(Self {
            state: Store::new(State::default()),
            commands: cmd_tx,
        });

        let state = this.state.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = msg_rx.recv().await {
                state.update(|s| match msg {
                    Message::Upsert(e) => {
                        s.endpoints.insert(e.id, e);
                    }
                    Message::Remove(id) => {
                        s.endpoints.remove(&id);
                    }
                    Message::DefaultSpeaker(name) => s.default_speaker = name,
                    Message::DefaultMicrophone(name) => s.default_microphone = name,
                    Message::Reset => *s = State::default(),
                });
            }
        });

        this
    }

    pub fn set_volume(&self, id: u32, volume: f64) {
        let _ = self.commands.send(Command::SetVolume(id, volume));
    }

    pub fn set_mute(&self, id: u32, mute: bool) {
        let _ = self.commands.send(Command::SetMute(id, mute));
    }
}

// ---------------------------------------------------------------------------
// PipeWire thread
// ---------------------------------------------------------------------------

struct NodeEntry {
    proxy: Node,
    _listener: Option<NodeListener>,
    endpoint: Endpoint,
    channels: usize,
    has_props: bool,
    /// Device id and route device index, for nodes backed by a sound card.
    device: Option<(u32, i32)>,
}

struct Route {
    index: i32,
    device: i32,
}

struct DeviceEntry {
    proxy: Device,
    _listener: Option<DeviceListener>,
    routes: Vec<Route>,
}

struct Inner {
    tx: async_channel::Sender<Message>,
    nodes: HashMap<u32, NodeEntry>,
    devices: HashMap<u32, DeviceEntry>,
    metadata: Option<(u32, Metadata, MetadataListener)>,
}

impl Inner {
    fn send(&self, msg: Message) {
        let _ = self.tx.send_blocking(msg);
    }

    fn publish(&self, id: u32) {
        if let Some(node) = self.nodes.get(&id).filter(|n| n.has_props) {
            self.send(Message::Upsert(node.endpoint.clone()));
        }
    }
}

fn serialize(object: Object) -> Option<Vec<u8>> {
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(object))
        .ok()
        .map(|(cursor, _)| cursor.into_inner())
}

fn parse_object(pod: &Pod) -> Option<Object> {
    match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, Value::Object(object))) => Some(object),
        _ => None,
    }
}

fn describe(props: &DictRef, kind: Kind) -> String {
    let keys: &[&str] = match kind {
        Kind::Stream => &[
            "application.name",
            "node.description",
            "media.name",
            "node.name",
        ],
        _ => &["node.description", "node.nick", "node.name"],
    };
    keys.iter()
        .find_map(|k| props.get(k).filter(|v| !v.is_empty()))
        .unwrap_or_default()
        .to_owned()
}

fn on_node_info(inner: &Weak<RefCell<Inner>>, id: u32, props: Option<&DictRef>) {
    let (Some(inner), Some(props)) = (inner.upgrade(), props) else {
        return;
    };
    let mut inner = inner.borrow_mut();
    let Some(node) = inner.nodes.get_mut(&id) else {
        return;
    };
    // Guard against partial property sets; never forget a known name.
    if let Some(name) = props.get("node.name").filter(|n| !n.is_empty()) {
        node.endpoint.name = name.to_owned();
    }
    let description = describe(props, node.endpoint.kind);
    if !description.is_empty() {
        node.endpoint.description = description;
    }
    let device = props.get("device.id").and_then(|d| d.parse().ok()).zip(
        props
            .get("card.profile.device")
            .and_then(|d| d.parse().ok()),
    );
    if device.is_some() {
        node.device = device;
    }
    inner.publish(id);
}

fn on_node_props(inner: &Weak<RefCell<Inner>>, id: u32, pod: Option<&Pod>) {
    let (Some(inner), Some(object)) = (inner.upgrade(), pod.and_then(parse_object)) else {
        return;
    };
    let mut inner = inner.borrow_mut();
    let Some(node) = inner.nodes.get_mut(&id) else {
        return;
    };

    let mut changed = false;
    for prop in object.properties {
        match (prop.key, prop.value) {
            (spa::sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v)))
                if !v.is_empty() =>
            {
                let avg = v.iter().map(|&c| c as f64).sum::<f64>() / v.len() as f64;
                node.endpoint.volume = avg.cbrt();
                node.channels = v.len();
                changed = true;
            }
            (spa::sys::SPA_PROP_mute, Value::Bool(mute)) => {
                node.endpoint.mute = mute;
                changed = true;
            }
            _ => {}
        }
    }

    if changed {
        node.has_props = true;
        inner.publish(id);
    }
}

fn on_device_route(inner: &Weak<RefCell<Inner>>, id: u32, pod: Option<&Pod>) {
    let (Some(inner), Some(object)) = (inner.upgrade(), pod.and_then(parse_object)) else {
        return;
    };
    let mut index = None;
    let mut device = None;
    for prop in object.properties {
        match (prop.key, prop.value) {
            (spa::sys::SPA_PARAM_ROUTE_index, Value::Int(i)) => index = Some(i),
            (spa::sys::SPA_PARAM_ROUTE_device, Value::Int(d)) => device = Some(d),
            _ => {}
        }
    }
    let (Some(index), Some(device)) = (index, device) else {
        return;
    };
    if let Some(entry) = inner.borrow_mut().devices.get_mut(&id) {
        entry.routes.retain(|r| r.device != device);
        entry.routes.push(Route { index, device });
    }
}

fn on_metadata(inner: &Weak<RefCell<Inner>>, subject: u32, key: Option<&str>, value: Option<&str>) {
    // Defaults live on the core subject. Other subjects hold per-node entries
    // (e.g. a stream's `target.object`); when such a node goes away its
    // entries are cleared with a `None` key, which must not wipe the defaults.
    if subject != pw::core::PW_ID_CORE {
        return;
    }
    let Some(inner) = inner.upgrade() else {
        return;
    };
    let inner = inner.borrow();
    let name = || {
        let json: serde_json::Value = serde_json::from_str(value?).ok()?;
        json.get("name")?.as_str().map(str::to_owned)
    };
    match key {
        Some("default.audio.sink") => inner.send(Message::DefaultSpeaker(name())),
        Some("default.audio.source") => inner.send(Message::DefaultMicrophone(name())),
        // A `None` key clears all metadata.
        None => {
            inner.send(Message::DefaultSpeaker(None));
            inner.send(Message::DefaultMicrophone(None));
        }
        _ => {}
    }
}

fn add_node(
    inner_rc: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&DictRef>,
    kind: Kind,
) {
    let Ok(proxy) = registry.bind::<Node, _>(global) else {
        return;
    };
    let id = global.id;
    let weak = Rc::downgrade(inner_rc);
    let weak2 = weak.clone();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            // Info events for e.g. state changes carry no properties.
            if info.change_mask().contains(NodeChangeMask::PROPS) {
                on_node_info(&weak, id, info.props());
            }
        })
        .param(move |_, param, _, _, pod| {
            if param == ParamType::Props {
                on_node_props(&weak2, id, pod);
            }
        })
        .register();
    proxy.subscribe_params(&[ParamType::Props]);

    inner_rc.borrow_mut().nodes.insert(
        id,
        NodeEntry {
            proxy,
            _listener: Some(listener),
            endpoint: Endpoint {
                id,
                kind,
                name: String::new(),
                description: String::new(),
                volume: 0.0,
                mute: false,
            },
            channels: 0,
            has_props: false,
            device: None,
        },
    );
}

fn add_device(
    inner_rc: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&DictRef>,
) {
    let Ok(proxy) = registry.bind::<Device, _>(global) else {
        return;
    };
    let id = global.id;
    let weak = Rc::downgrade(inner_rc);
    let listener = proxy
        .add_listener_local()
        .param(move |_, param, _, _, pod| {
            if param == ParamType::Route {
                on_device_route(&weak, id, pod);
            }
        })
        .register();
    proxy.subscribe_params(&[ParamType::Route]);

    inner_rc.borrow_mut().devices.insert(
        id,
        DeviceEntry {
            proxy,
            _listener: Some(listener),
            routes: Vec::new(),
        },
    );
}

fn add_metadata(
    inner_rc: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&DictRef>,
) {
    let Ok(proxy) = registry.bind::<Metadata, _>(global) else {
        return;
    };
    let weak = Rc::downgrade(inner_rc);
    let listener = proxy
        .add_listener_local()
        .property(move |subject, key, _type, value| {
            on_metadata(&weak, subject, key, value);
            0
        })
        .register();
    inner_rc.borrow_mut().metadata = Some((global.id, proxy, listener));
}

fn on_global(
    inner: &Rc<RefCell<Inner>>,
    registry: &pw::registry::RegistryRc,
    global: &GlobalObject<&DictRef>,
) {
    let Some(props) = global.props else {
        return;
    };
    match global.type_ {
        ObjectType::Node => {
            if let Some(kind) = props.get("media.class").and_then(Kind::from_media_class) {
                add_node(inner, registry, global, kind);
            }
        }
        ObjectType::Device if props.get("media.class") == Some("Audio/Device") => {
            add_device(inner, registry, global);
        }
        ObjectType::Metadata if props.get("metadata.name") == Some("default") => {
            add_metadata(inner, registry, global);
        }
        _ => {}
    }
}

fn on_global_remove(inner: &Rc<RefCell<Inner>>, id: u32) {
    let mut inner = inner.borrow_mut();
    if inner.nodes.remove(&id).is_some() {
        inner.send(Message::Remove(id));
    }
    inner.devices.remove(&id);
    if inner
        .metadata
        .as_ref()
        .is_some_and(|(mid, _, _)| *mid == id)
    {
        inner.metadata = None;
        inner.send(Message::DefaultSpeaker(None));
        inner.send(Message::DefaultMicrophone(None));
    }
}

fn apply(inner: &Inner, command: Command) {
    let (id, volume, mute) = match command {
        Command::SetVolume(id, v) => (id, Some(v), None),
        Command::SetMute(id, m) => (id, None, Some(m)),
    };
    let Some(node) = inner.nodes.get(&id) else {
        return;
    };

    let mut props = Vec::new();
    if let Some(volume) = volume {
        let linear = volume.max(0.0).powi(3) as f32;
        props.push(Property::new(
            spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![linear; node.channels.max(1)])),
        ));
    }
    if let Some(mute) = mute {
        props.push(Property::new(spa::sys::SPA_PROP_mute, Value::Bool(mute)));
    }

    // Hardware nodes are controlled through their device's active route so
    // the change is persisted, the same way WirePlumber and pipewire-pulse do.
    let route = node.device.and_then(|(device_id, route_device)| {
        let device = inner.devices.get(&device_id)?;
        let route = device.routes.iter().find(|r| r.device == route_device)?;
        Some((device, route))
    });

    if let Some((device, route)) = route {
        let object = Object {
            type_: spa::sys::SPA_TYPE_OBJECT_ParamRoute,
            id: spa::sys::SPA_PARAM_Route,
            properties: vec![
                Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(route.index)),
                Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(route.device)),
                Property::new(
                    spa::sys::SPA_PARAM_ROUTE_props,
                    Value::Object(Object {
                        type_: spa::sys::SPA_TYPE_OBJECT_Props,
                        id: spa::sys::SPA_PARAM_Route,
                        properties: props,
                    }),
                ),
                Property::new(spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
            ],
        };
        if let Some(bytes) = serialize(object)
            && let Some(pod) = Pod::from_bytes(&bytes)
        {
            device.proxy.set_param(ParamType::Route, 0, pod);
        }
    } else {
        let object = Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Props,
            id: spa::sys::SPA_PARAM_Props,
            properties: props,
        };
        if let Some(bytes) = serialize(object)
            && let Some(pod) = Pod::from_bytes(&bytes)
        {
            node.proxy.set_param(ParamType::Props, 0, pod);
        }
    }
}

/// Runs until the connection to PipeWire is lost, then hands the command
/// receiver back so the caller can reconnect.
fn run(
    tx: &async_channel::Sender<Message>,
    commands: pw::channel::Receiver<Command>,
) -> Result<pw::channel::Receiver<Command>, pw::Error> {
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = match context.connect_rc(None) {
        Ok(core) => core,
        Err(e) => {
            eprintln!("pipewire: failed to connect: {e}");
            return Ok(commands);
        }
    };
    let registry = core.get_registry_rc()?;

    let inner = Rc::new(RefCell::new(Inner {
        tx: tx.clone(),
        nodes: HashMap::new(),
        devices: HashMap::new(),
        metadata: None,
    }));

    let loop_weak = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _seq, res, message| {
            eprintln!("pipewire: error on {id}: {res} {message}");
            if id == pw::core::PW_ID_CORE
                && let Some(main_loop) = loop_weak.upgrade()
            {
                main_loop.quit();
            }
        })
        .register();

    let registry_weak = registry.downgrade();
    let inner_global = inner.clone();
    let inner_remove = inner.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(registry) = registry_weak.upgrade() {
                on_global(&inner_global, &registry, global);
            }
        })
        .global_remove(move |id| on_global_remove(&inner_remove, id))
        .register();

    let inner_cmd = inner.clone();
    let attached = commands.attach(main_loop.loop_(), move |command| {
        apply(&inner_cmd.borrow(), command);
    });

    main_loop.run();

    let commands = attached.deattach();
    // Drop proxies before the core they belong to.
    {
        let mut inner = inner.borrow_mut();
        inner.nodes.clear();
        inner.devices.clear();
        inner.metadata = None;
    }
    Ok(commands)
}
