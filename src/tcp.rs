use crate::*;

async fn drain_receiver<T>(mut rx: mpsc::Receiver<T>) {
    while let Some(_) = rx.recv().await {}
}

pub async fn handle_tcp_write_end<O>(
    _connection_id: ConnectionId,
    mut rx: mpsc::Receiver<DispatcherMessage>,
    mut tcp_out: O,
) where
    O: tokio::io::AsyncWrite + std::marker::Unpin,
{
    while let Some(DispatcherMessage::IncomingData(d, _seq_num)) = rx.recv().await {
        log!(
            "{} writing seq {}, {} bytes",
            _connection_id,
            _seq_num,
            d.len()
        );
        if tcp_out.write_all(&d).await.is_err() {
            log!(
                "{} write error, discarding all further messages",
                _connection_id
            );
            tokio::spawn(async move { drain_receiver(rx).await });
            break;
        }
    }
    log!("{} shutting down write end", _connection_id);
    if let Err(_e) = tcp_out.shutdown().await {
        log!("{} shutdown failed: {:?}", _connection_id, _e);
    }
}

pub async fn handle_tcp_read_end<I>(
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
