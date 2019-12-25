use bytes::{Bytes, BytesMut};
use futures::sink::SinkExt;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::stream::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{FramedRead, FramedWrite};

mod codec;
use codec::*;

// trace_macros!{true};
macro_rules! log {
    ($fmt:expr) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT })
    };
    ($fmt:expr, $($args:expr),*) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT }, $($args),*)
    };
}

const BUF_SIZE: usize = 2048;
type ConnectionId = u32;

#[derive(Debug, PartialEq)]
pub enum DispatcherMessage {
    CloseConnection,
    IncomingData(Bytes, u32),
}

pub struct DispatcherState {
    next_connection: ConnectionId,
    connections: HashMap<ConnectionId, mpsc::Sender<DispatcherMessage>>,
}

#[derive(Debug)]
pub struct DispatcherTunnelMessage {
    id: ConnectionId,
    payload: DispatcherMessage,
}

struct Dispatcher {
    state: Mutex<DispatcherState>,
    channel: mpsc::Sender<DispatcherTunnelMessage>,
}

async fn into_future<A>(a: A) -> A {
    a
}

impl Dispatcher {
    fn new(tunnel_channel: mpsc::Sender<DispatcherTunnelMessage>) -> Self {
        Dispatcher {
            state: Mutex::new(DispatcherState {
                next_connection: 0,
                connections: HashMap::new(),
            }),
            channel: tunnel_channel,
        }
    }

    async fn register_connection(&self) -> (ConnectionId, mpsc::Receiver<DispatcherMessage>) {
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

    async fn dispatch_message<F, FF, FFE>(
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

    async fn dispatch_message_to_incoming(&self, msg: DispatcherTunnelMessage) -> Result<(), ()> {
        self.dispatch_message(msg, |m| {
            log!("{} refusing to open missing connection", m.id);
            into_future(Err(()))
        })
        .await
    }

    async fn dispatch_message_to_outgoing(
        &self,
        msg: DispatcherTunnelMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        async fn missing_chan_cb(
            connection_id: ConnectionId,
            mut dispatcher_channel: mpsc::Sender<DispatcherTunnelMessage>,
        ) -> Result<mpsc::Sender<DispatcherMessage>, Box<dyn std::error::Error>> {
            let (tx, mut rx) = mpsc::channel::<DispatcherMessage>(10);
            log!("{} opening new outgoing connection", connection_id);
            let tcp =
                TcpStream::connect("127.0.0.1:8000".parse::<std::net::SocketAddrV4>().unwrap())
                    .await?;

            let (mut tcp_in, mut tcp_out) = tokio::io::split(tcp);

            //handle the outgoing tcp side
            tokio::spawn(async move {
                while let Some(DispatcherMessage::IncomingData(d, seq_num)) = rx.recv().await {
                    log!(
                        "{} writing seq {}, {} bytes",
                        connection_id,
                        seq_num,
                        d.len()
                    );
                    if tcp_out.write_all(&d).await.is_err() {
                        log!("{} write error", connection_id);
                        break;
                    }
                }
                log!("{} shutting down outgoing", connection_id);
                if let Err(e) = tcp_out.shutdown().await {
                    log!("{} shutdown failed: {:?}", connection_id, e);
                }
            });

            //handle the incoming tcp side
            tokio::spawn(async move {
                let mut bytes = BytesMut::with_capacity(BUF_SIZE);
                let mut seq_count = 0;
                while let Ok(bytes_read) = tcp_in.read_buf(&mut bytes).await {
                    log!(
                        "{} read input seq {}, {} bytes",
                        connection_id,
                        seq_count,
                        bytes_read
                    );
                    if bytes_read == 0 {
                        break;
                    }
                    dispatcher_channel
                        .send(DispatcherTunnelMessage {
                            id: connection_id,
                            payload: DispatcherMessage::IncomingData(
                                Bytes::copy_from_slice(&bytes),
                                seq_count,
                            ),
                        })
                        .await
                        .unwrap();
                    seq_count += 1;
                    bytes.clear();
                }
                log!("{} stream end, telling to close connection", connection_id);
                dispatcher_channel
                    .send(DispatcherTunnelMessage {
                        id: connection_id,
                        payload: DispatcherMessage::CloseConnection,
                    })
                    .await
                    .unwrap();
            });

            Ok(tx)
        }

        self.dispatch_message(msg, |m| missing_chan_cb(m.id, self.channel.clone()))
            .await
    }

    async fn handle_connection(&self, con: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
        let (connection_id, mut dispatcher_channel_read) = self.register_connection().await;
        log!("{} registering connection", connection_id);
        let (mut tcp_in, mut tcp_out) = tokio::io::split(con);
        let mut dispatcher_channel_write = self.channel.clone();

        let input_future = tokio::spawn(async move {
            let mut bytes = BytesMut::with_capacity(BUF_SIZE);
            let mut seq_count = 0;
            while let Ok(bytes_read) = tcp_in.read_buf(&mut bytes).await {
                if bytes_read == 0 {
                    break;
                }
                log!(
                    "{} read input: seq {}, {} bytes",
                    connection_id,
                    seq_count,
                    bytes.len()
                );
                dispatcher_channel_write
                    .send(DispatcherTunnelMessage {
                        id: connection_id,
                        payload: DispatcherMessage::IncomingData(
                            Bytes::copy_from_slice(&bytes),
                            seq_count,
                        ),
                    })
                    .await?;
                seq_count += 1;
                bytes.clear();
            }
            log!("{} stream end, telling to close connection", connection_id);

            dispatcher_channel_write
                .send(DispatcherTunnelMessage {
                    id: connection_id,
                    payload: DispatcherMessage::CloseConnection,
                })
                .await?;
            Ok::<(), tokio::sync::mpsc::error::SendError<DispatcherTunnelMessage>>(())
        });

        let mut continue_loop = true;
        while continue_loop {
            continue_loop = false;
            match dispatcher_channel_read.recv().await {
                Some(DispatcherMessage::IncomingData(data, seq_num)) => {
                    log!(
                        "{} writing seq {} {} bytes",
                        connection_id,
                        seq_num,
                        data.len()
                    );
                    tcp_out.write_all(&data).await?;
                    continue_loop = true;
                }
                _ => {
                    log!("{} shuting down", connection_id);
                    tcp_out.shutdown().await?;
                }
            }
        }

        input_future.await??;
        Ok(())
    }
}

use tokio::process::Child;
#[allow(clippy::mut_from_ref)]
fn get_stdin(child: &Child) -> &mut tokio::process::ChildStdin {
    let ptr: *const Child = child;
    unsafe {
        (ptr as *mut Child)
            .as_mut()
            .unwrap()
            .stdin()
            .as_mut()
            .unwrap()
    }
}

#[allow(clippy::mut_from_ref)]
fn get_stdout(child: &Child) -> &mut tokio::process::ChildStdout {
    let ptr: *const Child = child;
    unsafe {
        (ptr as *mut Child)
            .as_mut()
            .unwrap()
            .stdout()
            .as_mut()
            .unwrap()
    }
}

async fn run_incoming(progname: String) -> Result<(), Box<dyn std::error::Error>> {
    log!("Running incoming");
    let addr: std::net::SocketAddrV4 = "0.0.0.0:8001".parse().unwrap();

    let (tx, mut rx) = mpsc::channel::<DispatcherTunnelMessage>(10);
    let dispatcher = Arc::new(Dispatcher::new(tx));
    let dispatcher_inner = dispatcher.clone();

    tokio::spawn(async move {
        let child = Arc::new(
            tokio::process::Command::new(progname)
                .arg("--outgoing")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let child_inner = child.clone();

        let pipe_write = get_stdin(&child);
        let mut framed_tunnel_write = FramedWrite::new(pipe_write, TunnelCodec::new());

        tokio::spawn(async move {
            let pipe_read = get_stdout(&child_inner);
            let mut framed_tunnel_read = FramedRead::new(pipe_read, TunnelCodec::new());
            loop {
                let val = framed_tunnel_read.next().await.unwrap().unwrap();
                dispatcher_inner
                    .dispatch_message_to_incoming(val)
                    .await
                    .unwrap();
            }
        });

        loop {
            let val = rx.recv().await.unwrap();
            // log!("{} into tunnel: {:?}", val.id, val.payload);
            framed_tunnel_write.send(val).await.unwrap();
        }
        // this is a kind of type annotation
        // Ok::<(), std::io::Error>(())
    });

    let mut listener = TcpListener::bind(&addr).await?;

    loop {
        let (sock, _) = listener.accept().await?;
        let disp = dispatcher.clone();
        tokio::spawn(async move {
            disp.handle_connection(sock).await.unwrap();
        });
    }
}

async fn run_outgoing() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        IN_OR_OUT = "outgoing >>>";
    }
    log!("Running outgoing");

    let mut pipe_in = FramedRead::new(tokio::io::stdin(), TunnelCodec::new());

    let (mut tx, mut rx) = mpsc::channel::<DispatcherTunnelMessage>(10);
    let dispatcher = Arc::new(Dispatcher::new(tx.clone()));

    tokio::spawn(async move {
        let mut pipe_out = FramedWrite::new(tokio::io::stdout(), TunnelCodec::new());
        while let Some(d) = rx.recv().await {
            // log!("{} into tunnel: {:?}",d.id, d);
            pipe_out.send(d).await.unwrap();
        }
    });

    while let Some(Ok(d)) = pipe_in.next().await {
        let id = d.id;
        if dispatcher.dispatch_message_to_outgoing(d).await.is_err() {
            log!("{} could not establish connection. terminating", id);
            tx.send(DispatcherTunnelMessage {
                id,
                payload: DispatcherMessage::CloseConnection,
            })
            .await
            .unwrap();
        };
    }
    Ok(())
}

static mut IN_OR_OUT: &str = "incoming <<<"; //>= std::cell::RefCell::new("incoming");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let prog = args.next().unwrap();
    match args.next() {
        Some(s) => match s.as_ref() {
            "--outgoing" => run_outgoing().await,
            _ => run_incoming(prog).await,
        },
        _ => run_incoming(prog).await,
    }
}
