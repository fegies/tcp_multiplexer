use crate::*;

async fn dispatch_message_to_outgoing(
    dispatcher: &Dispatcher,
    target_addr: &std::net::SocketAddr,
    msg: DispatcherTunnelMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    async fn missing_chan_cb(
        connection_id: ConnectionId,
        target_addr: &std::net::SocketAddr,
        dispatcher_channel: mpsc::Sender<DispatcherTunnelMessage>,
    ) -> Result<mpsc::Sender<DispatcherMessage>, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel::<DispatcherMessage>(10);
        log!("{} opening new outgoing connection", connection_id);
        let tcp =
            TcpStream::connect(target_addr).await?;

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
        .dispatch_message(msg, |m| missing_chan_cb(m.id, target_addr, dispatcher.channel.clone()))
        .await
}

pub async fn run_outgoing(target_addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    log!("Running outgoing");

    let mut pipe_in = FramedRead::new(tokio::io::stdin(), TunnelCodec::new());

    let (mut tx, mut rx) = mpsc::channel::<DispatcherTunnelMessage>(10);
    let dispatcher = Arc::new(Dispatcher::new(tx.clone()));

    tokio::spawn(async move {
        let mut pipe_out = FramedWrite::new(tokio::io::stdout(), TunnelCodec::new());
        while let Some(d) = rx.recv().await {
            pipe_out.send(d).await.unwrap();
        }
    });

    while let Some(Ok(d)) = pipe_in.next().await {
        let id = d.id;
        if dispatch_message_to_outgoing(&dispatcher, &target_addr, d).await.is_err() {
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
