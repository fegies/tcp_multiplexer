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

pub async fn run_incoming(
    port: u16,
    prog: String,
    args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("Running incoming");
    let addr_str = format!("0.0.0.0:{}", port);
    let addr: std::net::SocketAddrV4 = addr_str.parse().unwrap();

    let (tx, mut rx) = mpsc::channel::<DispatcherTunnelMessage>(10);
    let dispatcher = Arc::new(Dispatcher::new(tx));
    let dispatcher_inner = dispatcher.clone();

    //set up the tunnel
    tokio::spawn(async move {
        let child = tokio::process::Command::new(prog)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let (child_out, child_in) = crate::child_split::split_child(child).unwrap();

        let mut framed_tunnel_write = FramedWrite::new(child_in, TunnelCodec::new());

        tokio::spawn(async move {
            let mut framed_tunnel_read = FramedRead::new(child_out, TunnelCodec::new());
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
