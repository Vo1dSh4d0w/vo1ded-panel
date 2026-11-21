//! Notification daemon implementing the freedesktop Desktop Notifications
//! specification (`org.freedesktop.Notifications`, version 1.2).
//!
//! Notifications are kept in a [`Store`]; the popups in
//! `widgets/notifications.rs` render it and report user interaction back
//! through [`Notifications::invoke`] and [`Notifications::dismiss`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib;

use crate::store::Store;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

const INTROSPECTION: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities">
      <arg direction="out" name="capabilities" type="as"/>
    </method>
    <method name="Notify">
      <arg direction="in" name="app_name" type="s"/>
      <arg direction="in" name="replaces_id" type="u"/>
      <arg direction="in" name="app_icon" type="s"/>
      <arg direction="in" name="summary" type="s"/>
      <arg direction="in" name="body" type="s"/>
      <arg direction="in" name="actions" type="as"/>
      <arg direction="in" name="hints" type="a{sv}"/>
      <arg direction="in" name="expire_timeout" type="i"/>
      <arg direction="out" name="id" type="u"/>
    </method>
    <method name="CloseNotification">
      <arg direction="in" name="id" type="u"/>
    </method>
    <method name="GetServerInformation">
      <arg direction="out" name="name" type="s"/>
      <arg direction="out" name="vendor" type="s"/>
      <arg direction="out" name="version" type="s"/>
      <arg direction="out" name="spec_version" type="s"/>
    </method>
    <signal name="NotificationClosed">
      <arg name="id" type="u"/>
      <arg name="reason" type="u"/>
    </signal>
    <signal name="ActionInvoked">
      <arg name="id" type="u"/>
      <arg name="action_key" type="s"/>
    </signal>
  </interface>
</node>
"#;

const CAPABILITIES: &[&str] = &[
    "actions",
    "action-icons",
    "body",
    "body-hyperlinks",
    "body-markup",
    "icon-static",
];

/// Timeout for notifications that leave it to the server (`expire_timeout`
/// of -1). Critical notifications never expire.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Action key invoked by clicking the notification itself; never shown as a
/// button.
pub const DEFAULT_ACTION: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Image {
    /// Raw pixels from the `image-data` hint.
    Data {
        width: i32,
        height: i32,
        rowstride: i32,
        has_alpha: bool,
        data: glib::Bytes,
    },
    /// An image file.
    File(String),
    /// A themed icon name.
    Icon(String),
}

impl Image {
    /// Parses an `image-path` hint or `app_icon`: a `file://` URI, an
    /// absolute path, or an icon name.
    fn from_path(path: &str) -> Option<Self> {
        if path.is_empty() {
            None
        } else if let Some(file) = path.strip_prefix("file://") {
            Some(Self::File(
                glib::Uri::unescape_string(file, None::<&str>).map_or(file.to_owned(), Into::into),
            ))
        } else if path.starts_with('/') {
            Some(Self::File(path.to_owned()))
        } else {
            Some(Self::Icon(path.to_owned()))
        }
    }

    /// Parses an `image-data` (`(iiibiiay)`) hint. Only 8 bits per sample
    /// RGB(A) data is defined by the spec.
    fn from_data(value: &glib::Variant) -> Option<Self> {
        let (width, height, rowstride, has_alpha, bits, channels, data) =
            value.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
        let expected_channels = if has_alpha { 4 } else { 3 };
        let valid = width > 0
            && height > 0
            && bits == 8
            && channels == expected_channels
            && rowstride >= width * channels
            // The last row may be unpadded.
            && data.len() as i64 >= (height as i64 - 1) * rowstride as i64 + (width * channels) as i64;
        valid.then(|| Self::Data {
            width,
            height,
            rowstride,
            has_alpha,
            data: glib::Bytes::from_owned(data),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    /// Pango markup, already sanitized (see [`body_markup`]).
    pub body: String,
    /// Actions shown as buttons; the default action is in
    /// [`Self::has_default_action`] instead.
    pub actions: Vec<Action>,
    pub has_default_action: bool,
    /// Action keys are icon names, shown instead of the labels.
    pub action_icons: bool,
    pub image: Option<Image>,
    pub urgency: Urgency,
    /// Kept around after an action is invoked.
    pub resident: bool,
    /// Screen position the notification should point to.
    pub position: Option<(i32, i32)>,
    /// `None` if the notification never expires.
    pub timeout: Option<Duration>,
}

/// Why a notification was closed, as sent in `NotificationClosed`.
#[derive(Debug, Clone, Copy)]
enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

pub struct Notifications {
    /// Open notifications, newest first.
    pub state: Rc<Store<Vec<Notification>>>,
    connection: RefCell<Option<gio::DBusConnection>>,
    timers: RefCell<HashMap<u32, glib::SourceId>>,
    next_id: Cell<u32>,
}

thread_local! {
    static INSTANCE: Rc<Notifications> = Notifications::start();
}

pub fn get() -> Rc<Notifications> {
    INSTANCE.with(Rc::clone)
}

impl Notifications {
    fn start() -> Rc<Self> {
        let this = Rc::new(Self {
            state: Store::new(Vec::new()),
            connection: RefCell::default(),
            timers: RefCell::default(),
            next_id: Cell::new(1),
        });

        let weak = Rc::downgrade(&this);
        // Take over from any running daemon that allows it, and let others
        // take over from us in turn.
        // The name stays owned for the lifetime of the process.
        let _ = gio::bus_own_name(
            gio::BusType::Session,
            NAME,
            gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
            move |connection, _| {
                if let Some(this) = weak.upgrade() {
                    this.register(connection);
                }
            },
            |_, _| {},
            |connection, _| match connection {
                Some(_) => eprintln!(
                    "notifications: {NAME} is owned by another daemon that doesn't allow \
                     replacement (or it replaced us); notifications are disabled"
                ),
                None => eprintln!("notifications: no session bus; notifications are disabled"),
            },
        );

        this
    }

    fn register(self: &Rc<Self>, connection: gio::DBusConnection) {
        let info = gio::DBusNodeInfo::for_xml(INTROSPECTION)
            .expect("invalid introspection XML")
            .lookup_interface(INTERFACE)
            .expect("interface missing from introspection XML");
        let weak = Rc::downgrade(self);
        let registration = connection
            .register_object(PATH, &info)
            .method_call(move |_, _, _, _, method, params, invocation| {
                let Some(this) = weak.upgrade() else { return };
                this.method_call(method, &params, invocation);
            })
            .build();
        match registration {
            // Registered for the lifetime of the connection.
            Ok(_) => {
                self.connection.replace(Some(connection));
            }
            Err(e) => eprintln!("notifications: {PATH}: {e}"),
        }
    }

    fn method_call(
        self: &Rc<Self>,
        method: &str,
        params: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        match method {
            "GetCapabilities" => {
                let caps: Vec<String> = CAPABILITIES.iter().map(|&c| c.to_owned()).collect();
                invocation.return_value(Some(&(caps,).to_variant()));
            }
            "GetServerInformation" => {
                let info = ("vo1ded-panel", "vo1ded", env!("CARGO_PKG_VERSION"), "1.2");
                invocation.return_value(Some(&info.to_variant()));
            }
            "Notify" => {
                type Args = (
                    String,
                    u32,
                    String,
                    String,
                    String,
                    Vec<String>,
                    HashMap<String, glib::Variant>,
                    i32,
                );
                let Some(args) = params.get::<Args>() else {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "invalid Notify arguments",
                    );
                    return;
                };
                let id = self.notify(args);
                invocation.return_value(Some(&(id,).to_variant()));
            }
            "CloseNotification" => {
                if let Some((id,)) = params.get::<(u32,)>() {
                    self.close(id, Reason::Closed);
                }
                invocation.return_value(None);
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                &format!("unknown method {method}"),
            ),
        }
    }

    #[allow(clippy::type_complexity)]
    fn notify(
        self: &Rc<Self>,
        (app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout): (
            String,
            u32,
            String,
            String,
            String,
            Vec<String>,
            HashMap<String, glib::Variant>,
            i32,
        ),
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            let id = self.next_id.get();
            // 0 is reserved for "no notification".
            self.next_id.set(id.checked_add(1).unwrap_or(1));
            id
        };

        let hint = |key: &str| hints.get(key);
        let bool_hint = |key: &str| hint(key).and_then(|v| v.get::<bool>()).unwrap_or(false);
        let urgency = match hint("urgency").and_then(int_value) {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };

        // Priority as per the spec: image-data, image-path, app_icon, and
        // the deprecated icon_data last.
        let image = ["image-data", "image_data"]
            .iter()
            .find_map(|k| hint(k).and_then(Image::from_data))
            .or_else(|| {
                ["image-path", "image_path"]
                    .iter()
                    .find_map(|k| hint(k)?.get::<String>().and_then(|p| Image::from_path(&p)))
            })
            .or_else(|| Image::from_path(&app_icon))
            .or_else(|| hint("icon_data").and_then(Image::from_data));

        let position = hint("x")
            .and_then(int_value)
            .zip(hint("y").and_then(int_value));

        let mut has_default_action = false;
        let actions = actions
            .chunks(2)
            .filter_map(|pair| {
                let [key, label] = pair else { return None };
                if key == DEFAULT_ACTION {
                    has_default_action = true;
                    return None;
                }
                Some(Action {
                    key: key.clone(),
                    label: label.clone(),
                })
            })
            .collect();

        let timeout = match expire_timeout {
            0 => None,
            t if t > 0 => Some(Duration::from_millis(t as u64)),
            _ if urgency == Urgency::Critical => None,
            _ => Some(DEFAULT_TIMEOUT),
        };

        let notification = Notification {
            id,
            app_name,
            summary,
            body: body_markup(&body),
            actions,
            has_default_action,
            action_icons: bool_hint("action-icons"),
            image,
            urgency,
            resident: bool_hint("resident"),
            position,
            timeout,
        };

        self.state.update(|list| {
            // A replacement keeps its place in the stack.
            match list.iter().position(|n| n.id == id) {
                Some(i) => list[i] = notification,
                None => list.insert(0, notification),
            }
        });
        self.start_timer(id);
        id
    }

    /// (Re)starts the expiry timer of notification `id`.
    fn start_timer(self: &Rc<Self>, id: u32) {
        self.stop_timer(id);
        let timeout = self.state.get().iter().find(|n| n.id == id).and_then(|n| n.timeout);
        let Some(timeout) = timeout else { return };
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(timeout, move || {
            if let Some(this) = weak.upgrade() {
                // The source is finished; don't remove it again.
                this.timers.borrow_mut().remove(&id);
                this.close(id, Reason::Expired);
            }
        });
        self.timers.borrow_mut().insert(id, source);
    }

    fn stop_timer(&self, id: u32) {
        if let Some(source) = self.timers.borrow_mut().remove(&id) {
            source.remove();
        }
    }

    /// Keeps notification `id` from expiring, e.g. while it is hovered.
    pub fn hold(&self, id: u32) {
        self.stop_timer(id);
    }

    /// Undoes [`Self::hold`], restarting the full timeout.
    pub fn release(self: &Rc<Self>, id: u32) {
        if self.state.get().iter().any(|n| n.id == id) {
            self.start_timer(id);
        }
    }

    /// Closes notification `id` at the user's request.
    pub fn dismiss(&self, id: u32) {
        self.close(id, Reason::Dismissed);
    }

    /// Invokes action `key` of notification `id`, closing it unless it is
    /// resident.
    pub fn invoke(&self, id: u32, key: &str) {
        let Some(resident) = self.state.get().iter().find(|n| n.id == id).map(|n| n.resident)
        else {
            return;
        };
        self.emit("ActionInvoked", &(id, key).to_variant());
        if !resident {
            self.close(id, Reason::Dismissed);
        }
    }

    fn close(&self, id: u32, reason: Reason) {
        self.stop_timer(id);
        if !self.state.get().iter().any(|n| n.id == id) {
            return;
        }
        self.state.update(|list| list.retain(|n| n.id != id));
        self.emit("NotificationClosed", &(id, reason as u32).to_variant());
    }

    fn emit(&self, signal: &str, params: &glib::Variant) {
        let Some(connection) = self.connection.borrow().clone() else {
            return;
        };
        if let Err(e) = connection.emit_signal(None, PATH, INTERFACE, signal, Some(params)) {
            eprintln!("notifications: {signal}: {e}");
        }
    }
}

/// Reads an integer hint. The spec says which integer type each hint has,
/// but clients aren't consistent about it.
fn int_value(value: &glib::Variant) -> Option<i32> {
    value
        .get::<i32>()
        .or_else(|| value.get::<u8>().map(i32::from))
        .or_else(|| value.get::<i16>().map(i32::from))
        .or_else(|| value.get::<u16>().map(i32::from))
        .or_else(|| value.get::<u32>().and_then(|v| v.try_into().ok()))
        .or_else(|| value.get::<i64>().and_then(|v| v.try_into().ok()))
        .or_else(|| value.get::<u64>().and_then(|v| v.try_into().ok()))
}

/// Converts a notification body to Pango markup GTK labels accept.
///
/// The spec's markup is a small XML-ish subset (`<b>`, `<i>`, `<u>`,
/// `<a href>`, `<img alt>`), and clients often send unescaped `&` or `<`, or
/// unbalanced tags. Supported tags are kept, `<img>` becomes its alt text,
/// `<br>` a newline, and everything else is escaped as text.
pub fn body_markup(body: &str) -> String {
    let markup = sanitize(body);
    // Links are GtkLabel's addition to Pango markup, so Pango alone can only
    // check the markup with them as plain spans. `sanitize` writes them
    // exactly as `<a href="…">`, with the URL escaped.
    let links = regex::Regex::new(r#"<a href="[^"]*">"#).expect("valid regex");
    let without_links = links.replace_all(&markup, "<span>").replace("</a>", "</span>");
    if gtk::pango::parse_markup(&without_links, '\0').is_ok() {
        markup
    } else {
        glib::markup_escape_text(body).into()
    }
}

fn sanitize(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut open: Vec<&'static str> = Vec::new();
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        match c {
            '<' => {
                if let Some(end) = rest.find('>')
                    && tag(&rest[1..end], &mut open, &mut out)
                {
                    rest = &rest[end + 1..];
                    continue;
                }
                out.push_str("&lt;");
            }
            '>' => out.push_str("&gt;"),
            '&' => match entity_len(rest) {
                Some(len) => {
                    out.push_str(&rest[..len]);
                    rest = &rest[len..];
                    continue;
                }
                None => out.push_str("&amp;"),
            },
            c => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    while let Some(name) = open.pop() {
        out.push_str(&format!("</{name}>"));
    }
    out
}

/// Handles the tag between `<` and `>`, returning `false` if it isn't one
/// of the supported tags (so it is shown as text instead).
fn tag(tag: &str, open: &mut Vec<&'static str>, out: &mut String) -> bool {
    // Tag names follow `<` directly; `a < b > c` is text.
    if tag.starts_with(char::is_whitespace) {
        return false;
    }
    let tag = tag.trim_end();
    if let Some(name) = tag.strip_prefix('/') {
        let Some(name) = known(name.trim()) else {
            return false;
        };
        // Close everything opened since, keeping the markup well-formed;
        // stray closing tags are dropped.
        if let Some(i) = open.iter().rposition(|&n| n == name) {
            for n in open.drain(i..).rev() {
                out.push_str(&format!("</{n}>"));
            }
        }
        return true;
    }

    let tag = tag.strip_suffix('/').unwrap_or(tag);
    let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let (name, attrs) = tag.split_at(name_end);
    match name.to_ascii_lowercase().as_str() {
        "br" => out.push('\n'),
        "img" => {
            if let Some(alt) = attribute(attrs, "alt") {
                out.push_str(&glib::markup_escape_text(&alt));
            }
        }
        // Links can't nest.
        "a" if open.contains(&"a") => {}
        "a" => {
            if let Some(href) = attribute(attrs, "href") {
                out.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(&href)));
                open.push("a");
            }
        }
        _ => {
            let Some(name) = known(name) else {
                return false;
            };
            out.push_str(&format!("<{name}>"));
            open.push(name);
        }
    }
    true
}

/// Supported tags that can be opened and closed.
fn known(name: &str) -> Option<&'static str> {
    ["b", "i", "u", "a"]
        .into_iter()
        .find(|n| n.eq_ignore_ascii_case(name))
}

/// Value of attribute `name` in `attrs` (` key="value" key2='value'`), with
/// the basic XML entities unescaped.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let key = rest[..eq].trim();
        rest = rest[eq + 1..].trim_start();
        let value = match rest.chars().next()? {
            q @ ('"' | '\'') => {
                let end = rest[1..].find(q)? + 1;
                let value = &rest[1..end];
                rest = &rest[end + 1..];
                value
            }
            _ => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let value = &rest[..end];
                rest = &rest[end..];
                value
            }
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(
                value
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&apos;", "'")
                    .replace("&amp;", "&"),
            );
        }
    }
}

/// Length of the entity reference at the start of `s` (which starts with
/// `&`), if it is one Pango understands.
fn entity_len(s: &str) -> Option<usize> {
    let end = s.find(';')?;
    let name = &s[1..end];
    let valid = match name.strip_prefix('#') {
        Some(num) => match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        }
        .and_then(char::from_u32)
        .is_some_and(|c| c != '\0'),
        None => ["amp", "lt", "gt", "quot", "apos"].contains(&name),
    };
    valid.then_some(end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_supported_tags() {
        assert_eq!(sanitize("<b>bold</b> <I>it</I>"), "<b>bold</b> <i>it</i>");
    }

    #[test]
    fn escapes_stray_characters() {
        assert_eq!(sanitize("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        assert_eq!(sanitize("<3 <unknown>"), "&lt;3 &lt;unknown&gt;");
    }

    #[test]
    fn keeps_entities() {
        assert_eq!(sanitize("&amp; &#38; &#x26; &nbsp;"), "&amp; &#38; &#x26; &amp;nbsp;");
    }

    #[test]
    fn balances_tags() {
        assert_eq!(sanitize("<b><i>x</b> y</i>"), "<b><i>x</i></b> y");
        assert_eq!(sanitize("<u>open"), "<u>open</u>");
    }

    #[test]
    fn links_and_images() {
        assert_eq!(
            sanitize(r#"<a href="https://x.y/?a=1&amp;b=2">link</a>"#),
            r#"<a href="https://x.y/?a=1&amp;b=2">link</a>"#
        );
        assert_eq!(sanitize(r#"<img src="x.png" alt="pic & text"/>"#), "pic &amp; text");
        assert_eq!(sanitize("line<br/>next"), "line\nnext");
    }

    #[test]
    fn result_is_valid_pango_markup() {
        for body in ["<b>x", "</b>y", "<a>no href</a>", "&#0; <i></b>", "a<b"] {
            let markup = body_markup(body);
            assert!(gtk::pango::parse_markup(&markup, '\0').is_ok(), "{body:?} -> {markup:?}");
        }
    }

    #[test]
    fn links_survive_validation() {
        let body = r#"see <a href="https://example.com">this</a> & <b>that</b>"#;
        assert_eq!(
            body_markup(body),
            r#"see <a href="https://example.com">this</a> &amp; <b>that</b>"#
        );
    }
}
