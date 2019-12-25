use bytes::{Bytes, BytesMut};
use futures::sink::SinkExt;

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};

macro_rules! log {
    ($fmt:expr) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT })
    };
    ($fmt:expr, $($args:expr),*) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT }, $($args),*)
    };
}

mod dispatcher;
use dispatcher::*;
mod codec;
use codec::*;

const BUF_SIZE: usize = 2048;
type ConnectionId = u32;

#[derive(Debug, PartialEq)]
pub enum DispatcherMessage {
    CloseConnection,
    IncomingData(Bytes, u32),
}

#[derive(Debug)]
pub struct DispatcherTunnelMessage {
    id: ConnectionId,
    payload: DispatcherMessage,
}

async fn into_future<A>(a: A) -> A {
    a
}

async fn dispatch_message_to_incoming(
    dispatcher: &Dispatcher,
    msg: DispatcherTunnelMessage,
) -> Result<(), ()> {
    dispatcher
        .dispatch_message(msg, |m| {
            log!("{} refusing to open missing connection", m.id);
            into_future(Err(()))
        })
        .await
}

async fn handle_tcp_write_end<O>(
    connection_id: ConnectionId,
    mut rx: mpsc::Receiver<DispatcherMessage>,
    mut tcp_out: O,
) where
    O: tokio::io::AsyncWrite + std::marker::Unpin,
{
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
    log!("{} shutting down write end", connection_id);
    if let Err(e) = tcp_out.shutdown().await {
        log!("{} shutdown failed: {:?}", connection_id, e);
    }
}

async fn handle_tcp_read_end<I>(
    connection_id: ConnectionId,
    mut tcp_in: I,
    mut dispatcher_channel: mpsc::Sender<DispatcherTunnelMessage>,
) where
    I: tokio::io::AsyncRead,
{
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
                payload: DispatcherMessage::IncomingData(Bytes::copy_from_slice(&bytes), seq_count),
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
}

async fn dispatch_message_to_outgoing(
    dispatcher: &Dispatcher,
    msg: DispatcherTunnelMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    async fn missing_chan_cb(
        connection_id: ConnectionId,
        dispatcher_channel: mpsc::Sender<DispatcherTunnelMessage>,
    ) -> Result<mpsc::Sender<DispatcherMessage>, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel::<DispatcherMessage>(10);
        log!("{} opening new outgoing connection", connection_id);
        let tcp =
            TcpStream::connect("127.0.0.1:8000".parse::<std::net::SocketAddrV4>().unwrap()).await?;

        let (tcp_in, tcp_out) = tokio::io::split(tcp);

        //handle the outgoing tcp side
        tokio::spawn(async move { handle_tcp_write_end(connection_id, rx, tcp_out).await });

        //handle the incoming tcp side
        tokio::spawn(async move {
            handle_tcp_read_end(connection_id, tcp_in, dispatcher_channel).await;
        });

        Ok(tx)
    }

    dispatcher
        .dispatch_message(msg, |m| missing_chan_cb(m.id, dispatcher.channel.clone()))
        .await
}

async fn handle_connection(
    dispatcher: &Dispatcher,
    con: TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let (connection_id, dispatcher_channel_read) = dispatcher.register_connection().await;
    log!("{} registering connection", connection_id);
    let (tcp_in, tcp_out) = tokio::io::split(con);
    let dispatcher_channel_write = dispatcher.channel.clone();

    let input_future = tokio::spawn(async move {
        handle_tcp_read_end(connection_id, tcp_in, dispatcher_channel_write).await;
    });

    handle_tcp_write_end(connection_id, dispatcher_channel_read, tcp_out).await;

    input_future.await?;
    Ok(())
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
                dispatch_message_to_incoming(&dispatcher_inner, val)
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
            handle_connection(&disp, sock).await.unwrap();
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
        if dispatch_message_to_outgoing(&dispatcher, d).await.is_err() {
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
