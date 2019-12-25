use crate::*;

async fn into_future<A>(a: A) -> A {
    a
}

async fn dispatch_message_to_incoming(
    dispatcher: &Dispatcher,
    msg: DispatcherTunnelMessage,
) -> Result<(), ()> {
    dispatcher
        .dispatch_message(msg, |_m| {
            log!("{} refusing to open missing connection", _m.id);
            into_future(Err(()))
        })
        .await
}

async fn handle_incoming_connection(dispatcher: &Dispatcher, con: TcpStream) {
    let (connection_id, dispatcher_channel_read) = dispatcher.register_connection().await;
    log!("{} registering connection", connection_id);
    let (tcp_in, tcp_out) = tokio::io::split(con);
    let dispatcher_channel_write = dispatcher.channel.clone();

    tokio::spawn(async move {
        handle_tcp_read_end(connection_id, tcp_in, dispatcher_channel_write).await;
    });

    tokio::spawn(async move {
        handle_tcp_write_end(connection_id, dispatcher_channel_read, tcp_out).await;
    });
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

pub async fn run_incoming(prog: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    log!("Running incoming");
    let addr: std::net::SocketAddrV4 = "0.0.0.0:8001".parse().unwrap();

    let (tx, mut rx) = mpsc::channel::<DispatcherTunnelMessage>(10);
    let dispatcher = Arc::new(Dispatcher::new(tx));
    let dispatcher_inner = dispatcher.clone();

    //set up the tunnel
    tokio::spawn(async move {
        let child = Arc::new(
            tokio::process::Command::new(&prog[0])
                .args(prog.iter().skip(1))
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
        handle_incoming_connection(&dispatcher, sock).await;
    }
}
