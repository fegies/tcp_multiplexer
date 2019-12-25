use bytes::{Bytes, BytesMut};
use futures::sink::SinkExt;

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};
use structopt::StructOpt;

#[cfg(feature = "logging")]
macro_rules! log {
    ($fmt:expr) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT })
    };
    ($fmt:expr, $($args:expr),*) => {
        eprintln!(concat!("[{}] ", $fmt), unsafe { IN_OR_OUT }, $($args),*)
    };
}
#[cfg(not(feature = "logging"))]
macro_rules! log {
    ($($args:expr),*) => {};
}
static mut IN_OR_OUT: &str = "incoming <<<";

mod dispatcher;
use dispatcher::*;
mod codec;
use codec::*;
mod tcp;
use tcp::*;
mod incoming;
use incoming::*;
mod outgoing;
use outgoing::*;

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

#[derive(StructOpt, Debug)]
#[structopt(version="0.1", author="Felix Giese")]
enum Opts {
    TestBinary {},
    Incoming {
        #[structopt(short,long)]
        tunnel_command: Vec<String>
    },
    Outgoing {
        #[structopt(short,long)]
        target_connection: std::net::SocketAddr
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Opts::from_args() {
        Opts::TestBinary {} => {Ok(())},
        Opts::Incoming { tunnel_command: cmd } => {
            run_incoming(cmd).await
        },
        Opts::Outgoing { target_connection: target } => {
            unsafe {
                IN_OR_OUT = "outgoing >>>";
            }
            run_outgoing(target).await
        }
    }
    // let mut args = std::env::args();
    // let prog = args.next().unwrap();
    // match args.next() {
    //     Some(s) => match s.as_ref() {
    //         "--test-binary" => Ok(()),
    //         "--outgoing" => {
    //             unsafe {
    //                 IN_OR_OUT = "outgoing >>>";
    //             }
    //             run_outgoing().await
    //         }
    //         _ => run_incoming(prog).await,
    //     },
    //     _ => run_incoming(prog).await,
    // }
}
