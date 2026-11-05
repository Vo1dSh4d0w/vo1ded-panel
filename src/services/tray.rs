//! StatusNotifierItem tray. The `system-tray` client runs on its own Tokio
//! thread; updates are forwarded to the GTK main loop.

use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use system_tray::client::{ActivateRequest, Client, Event};
use system_tray::item::StatusNotifierItem;
use system_tray::menu::TrayMenu;
use tokio::sync::broadcast::error::RecvError;

use crate::store::Store;

#[derive(Debug, Clone)]
pub struct Item {
    /// D-Bus address, unique per item.
    pub address: String,
    pub item: StatusNotifierItem,
    pub menu: Option<TrayMenu>,
}

pub enum Command {
    Activate(ActivateRequest),
    AboutToShow { address: String, menu_path: String },
}

enum Message {
    Upsert(Box<Item>),
    Remove(String),
    Reset,
}

pub struct Tray {
    /// Items in the order they appeared.
    pub items: Rc<Store<Vec<Item>>>,
    commands: async_channel::Sender<Command>,
}

thread_local! {
    static INSTANCE: Rc<Tray> = Tray::start();
}

pub fn get() -> Rc<Tray> {
    INSTANCE.with(Rc::clone)
}

impl Tray {
    fn start() -> Rc<Self> {
        let (msg_tx, msg_rx) = async_channel::unbounded();
        let (cmd_tx, cmd_rx) = async_channel::unbounded();

        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to start tokio runtime");
                runtime.block_on(run(msg_tx, cmd_rx));
            })
            .expect("failed to spawn tray thread");

        let this = Rc::new(Self {
            items: Store::new(Vec::new()),
            commands: cmd_tx,
        });

        let items = this.items.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = msg_rx.recv().await {
                items.update(|items| match msg {
                    Message::Upsert(item) => {
                        match items.iter_mut().find(|i| i.address == item.address) {
                            Some(existing) => *existing = *item,
                            None => items.push(*item),
                        }
                    }
                    Message::Remove(address) => items.retain(|i| i.address != address),
                    Message::Reset => items.clear(),
                });
            }
        });

        this
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send_blocking(command);
    }
}

fn send_item(client: &Client, tx: &async_channel::Sender<Message>, address: &str) {
    let items = client.items();
    let items = items.lock().expect("tray items lock poisoned");
    let msg = match items.get(address) {
        Some((item, menu)) => Message::Upsert(Box::new(Item {
            address: address.to_owned(),
            item: item.clone(),
            menu: menu.clone(),
        })),
        None => Message::Remove(address.to_owned()),
    };
    let _ = tx.send_blocking(msg);
}

fn send_all(client: &Client, tx: &async_channel::Sender<Message>) {
    let _ = tx.send_blocking(Message::Reset);
    let addresses: Vec<String> = client
        .items()
        .lock()
        .expect("tray items lock poisoned")
        .keys()
        .cloned()
        .collect();
    for address in addresses {
        send_item(client, tx, &address);
    }
}

async fn run(tx: async_channel::Sender<Message>, commands: async_channel::Receiver<Command>) {
    let client = match Client::new().await {
        Ok(client) => Arc::new(client),
        Err(e) => {
            eprintln!("tray: failed to start: {e}");
            return;
        }
    };
    let mut events = client.subscribe();
    send_all(&client, &tx);

    let cmd_client = client.clone();
    tokio::spawn(async move {
        while let Ok(command) = commands.recv().await {
            let result = match command {
                Command::Activate(req) => cmd_client.activate(req).await.map(|_| ()),
                Command::AboutToShow { address, menu_path } => cmd_client
                    .about_to_show_menuitem(address, menu_path, 0)
                    .await
                    .map(|_| ()),
            };
            if let Err(e) = result {
                eprintln!("tray: {e}");
            }
        }
    });

    loop {
        match events.recv().await {
            Ok(Event::Add(address, _) | Event::Update(address, _)) => {
                send_item(&client, &tx, &address)
            }
            Ok(Event::Remove(address)) => {
                let _ = tx.send_blocking(Message::Remove(address));
            }
            Err(RecvError::Lagged(_)) => send_all(&client, &tx),
            Err(RecvError::Closed) => break,
        }
    }
}
