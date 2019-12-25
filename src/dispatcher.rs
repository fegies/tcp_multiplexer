use std::collections::hash_map::Entry;
use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::*;

struct DispatcherState {
    next_connection: ConnectionId,
    connections: HashMap<ConnectionId, mpsc::Sender<DispatcherMessage>>,
}

pub struct Dispatcher {
    state: Mutex<DispatcherState>,
    pub channel: mpsc::Sender<DispatcherTunnelMessage>,
}

impl Dispatcher {
    pub fn new(tunnel_channel: mpsc::Sender<DispatcherTunnelMessage>) -> Self {
        Dispatcher {
            state: Mutex::new(DispatcherState {
                next_connection: 0,
                connections: HashMap::new(),
            }),
            channel: tunnel_channel,
        }
    }

    pub async fn register_connection(&self) -> (ConnectionId, mpsc::Receiver<DispatcherMessage>) {
        let (tx, rx) = mpsc::channel(10);
        let mut guard = self.state.lock().await;
        let mut con = guard.next_connection;
        loop {
            guard.next_connection = guard.next_connection.wrapping_add(1);
            let entry = guard.connections.entry(con);
            match entry {
                Entry::Occupied(_) => {
                    con = guard.next_connection;
                    guard.next_connection = guard.next_connection.wrapping_add(1);
                }
                Entry::Vacant(ve) => {
                    ve.insert(tx);
                    break (con, rx);
                }
            }
        }
    }

    pub async fn dispatch_message<F, FF, FFE>(
        &self,
        msg: DispatcherTunnelMessage,
        missing_chan_cb: F,
    ) -> Result<(), FFE>
    where
        F: FnOnce(&DispatcherTunnelMessage) -> FF,
        FF: std::future::Future<Output = Result<mpsc::Sender<DispatcherMessage>, FFE>>,
    {
        let mut guard = self.state.lock().await;
        log!("{} dispatching message", msg.id);
        match msg.payload {
            DispatcherMessage::CloseConnection => {
                log!("{} got channel close message", msg.id);
                if let Some(mut chan) = guard.connections.remove(&msg.id) {
                    chan.send(msg.payload).await.unwrap();
                }
            }
            _ => match guard.connections.entry(msg.id) {
                Entry::Occupied(mut e) => e.get_mut().send(msg.payload).await.unwrap(),
                Entry::Vacant(e) => {
                    log!("{} no channel present, trying to get one", msg.id);
                    let chan = missing_chan_cb(&msg).await?;
                    e.insert(chan).send(msg.payload).await.unwrap();
                }
            },
        };
        Ok(())
    }
}
