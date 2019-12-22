use super::{ConnectionId, DispatcherMessage, DispatcherTunnelMessage};
use bytes::*;
use tokio_util::codec::*;

pub struct TunnelCodec {
    inner: length_delimited::LengthDelimitedCodec,
}

impl TunnelCodec {
    pub fn new() -> Self {
        TunnelCodec {
            inner: length_delimited::LengthDelimitedCodec::builder()
                .length_field_length(4)
                .length_adjustment(5)
                .new_codec(),
        }
    }
}

#[inline]
fn put_con_id(buf: &mut BytesMut, id: ConnectionId) {
    // buf.put_u8(id);
    buf.put_u32(id);
}

impl Encoder for TunnelCodec {
    type Item = DispatcherTunnelMessage;
    type Error = std::io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), std::io::Error> {
        let buf = match item.payload {
            DispatcherMessage::CloseConnection => {
                let mut buf = BytesMut::with_capacity(1 + 4 + std::mem::size_of::<ConnectionId>());
                buf.put_u8(0);
                put_con_id(&mut buf, item.id);
                buf.put_u32(0);
                buf
            }
            DispatcherMessage::IncomingData(data, seq_num) => {
                let mut buf = BytesMut::with_capacity(
                    data.len() + 1 + 4 + std::mem::size_of::<ConnectionId>(),
                );
                buf.put_u8(1);
                put_con_id(&mut buf, item.id);
                buf.put_u32(seq_num);
                buf.put(data);
                buf
            }
        };
        self.inner.encode(buf.freeze(), dst)
    }
}

impl Decoder for TunnelCodec {
    type Item = DispatcherTunnelMessage;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.inner.decode(src) {
            Ok(Some(mut data)) => {
                let status = data.get_u8();
                let id = data.get_u32();
                // let id = data.get_u16();
                // let id = data.get_u8();
                let seq_num = data.get_u32();
                let payload_r = match status {
                    0 => Ok(DispatcherMessage::CloseConnection),
                    1 => Ok(DispatcherMessage::IncomingData(data.to_bytes(), seq_num)),
                    _ => Err(std::io::Error::from(std::io::ErrorKind::InvalidData)),
                };
                payload_r.map(|payload| Some(DispatcherTunnelMessage { id, payload }))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
