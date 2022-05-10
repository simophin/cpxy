use anyhow::{bail, Context};
use byteorder::{BigEndian, WriteBytesExt};
use bytes::{Buf, Bytes};
use enum_primitive_derive::Primitive;
use futures::{Sink, SinkExt, Stream, StreamExt};
use lazy_static::lazy_static;
use num_traits::FromPrimitive;
use orion::aead::SecretKey;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::io::BytesRef;

type Nonce = u16;

#[derive(Debug, PartialEq, Eq)]
pub enum Message<'a> {
    Connect {
        uuid: BytesRef<'a>,
        initial_data: BytesRef<'a>,
        initial_data_nonce: Option<Nonce>,
        dst: SocketAddr,
    },

    Establish {
        uuid: Bytes,
        conn_id: u16,
        initial_reply: Option<(Bytes, SocketAddr)>,
    },

    Data {
        conn_id: Option<u16>,
        addr: Option<SocketAddr>,
        payload: BytesRef<'a>,
        enc_nonce: Option<Nonce>,
    },
}

#[derive(Primitive)]
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageType {
    Connect = 0,
    Establish = 1,
    Data = 2,
}

#[derive(Debug)]
enum MessageFlags {
    Connect {
        is_ipv6: bool,
        has_nonce: bool,
    },
    Establish {
        has_reply: bool,
        reply_in_ipv6: bool,
    },
    Data {
        has_conn_id: bool,
        has_addr: bool,
        addr_in_ipv6: bool,
        has_nonce: bool,
    },
}

impl TryFrom<u8> for MessageFlags {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(
            match MessageType::from_u8(value >> 5).context("Parsing message type")? {
                MessageType::Connect => Self::Connect {
                    is_ipv6: value & 0x1 != 0,
                    has_nonce: (value >> 1) & 0x1 != 0,
                },
                MessageType::Establish => Self::Establish {
                    has_reply: value & 0x1 != 0,
                    reply_in_ipv6: (value >> 1) & 0x1 != 0,
                },
                MessageType::Data => Self::Data {
                    has_conn_id: value & 0x1 != 0,
                    has_addr: (value >> 1) & 0x1 != 0,
                    addr_in_ipv6: (value >> 2) & 0x1 != 0,
                    has_nonce: (value >> 3) & 0x1 != 0,
                },
            },
        )
    }
}

impl Into<u8> for MessageFlags {
    fn into(self) -> u8 {
        match self {
            MessageFlags::Connect { is_ipv6, has_nonce } => {
                let mut flags = 0;
                if is_ipv6 {
                    flags |= 1;
                }
                if has_nonce {
                    flags |= 1 << 1;
                }
                ((MessageType::Connect as u8) << 5) | flags
            }
            MessageFlags::Establish {
                has_reply,
                reply_in_ipv6,
            } => {
                let mut flags = 0;
                if has_reply {
                    flags |= 1;
                }
                if reply_in_ipv6 {
                    flags |= 1 << 1;
                }
                ((MessageType::Establish as u8) << 5) | flags
            }
            MessageFlags::Data {
                has_conn_id,
                has_addr,
                addr_in_ipv6,
                has_nonce,
            } => {
                let mut flags = 0;
                if has_conn_id {
                    flags |= 1;
                }
                if has_addr {
                    flags |= 1 << 1;
                }
                if addr_in_ipv6 {
                    flags |= 1 << 2;
                }
                if has_nonce {
                    flags |= 1 << 3;
                }
                ((MessageType::Data as u8) << 5) | flags
            }
        }
    }
}

macro_rules! check_remaining {
    ($b:expr, $expected_len:expr, $ctx_str:expr) => {
        let remaining = $b.remaining();
        if remaining < $expected_len {
            bail!(
                "{}: Expecting {} bytes but only has {remaining} bytes remaining",
                $ctx_str,
                $expected_len
            )
        }
    };
}

fn parse_uuid(b: &mut Bytes) -> anyhow::Result<Bytes> {
    check_remaining!(b, 1, "UUID len");
    let uuid_len = b.get_u8() as usize;
    check_remaining!(b, uuid_len, "UUID");
    Ok(b.split_to(uuid_len))
}

fn write_uuid(w: &mut impl Write, id: &[u8]) -> anyhow::Result<()> {
    let len: u8 = id.len().try_into().context("Writing UUID len")?;
    w.write_u8(len)?;
    w.write_all(id)?;
    Ok(())
}

fn parse_socket_addr(b: &mut Bytes, v4: bool) -> anyhow::Result<SocketAddr> {
    let ip = if v4 {
        check_remaining!(b, 4, "IPv4 address");
        IpAddr::V4(Ipv4Addr::from(b.get_u32()))
    } else {
        check_remaining!(b, 16, "IPv6 address");
        IpAddr::V6(Ipv6Addr::from(b.get_u128()))
    };
    check_remaining!(b, 2, "Port");
    let port = b.get_u16();
    Ok(SocketAddr::new(ip, port))
}

fn write_socket_addr(w: &mut impl Write, addr: &SocketAddr) -> anyhow::Result<()> {
    match addr.ip() {
        IpAddr::V4(addr) => w.write_u32::<BigEndian>(addr.into())?,
        IpAddr::V6(addr) => w.write_u128::<BigEndian>(addr.into())?,
    };

    w.write_u16::<BigEndian>(addr.port())?;
    Ok(())
}

fn parse_conn_id(b: &mut Bytes) -> anyhow::Result<u16> {
    check_remaining!(b, 2, "Conn ID");
    Ok(b.get_u16())
}

impl<'a> Message<'a> {
    pub fn parse(mut b: Bytes) -> anyhow::Result<Self> {
        check_remaining!(b, 1, "MessageType");
        let flags = MessageFlags::try_from(b.get_u8()).context("Unknown message type")?;
        match flags {
            MessageFlags::Connect { is_ipv6, has_nonce } => {
                let uuid = parse_uuid(&mut b)?;
                let dst = parse_socket_addr(&mut b, !is_ipv6)?;
                let initial_data_nonce = if has_nonce {
                    check_remaining!(b, 2, "Nonce");
                    Some(b.get_u16())
                } else {
                    None
                };

                Ok(Self::Connect {
                    uuid: uuid.into(),
                    initial_data: b.into(),
                    initial_data_nonce,
                    dst,
                })
            }
            MessageFlags::Establish {
                has_reply,
                reply_in_ipv6,
            } => {
                let uuid = parse_uuid(&mut b)?;
                let conn_id = parse_conn_id(&mut b)?;

                if has_reply {
                    let from = parse_socket_addr(&mut b, !reply_in_ipv6)?;
                    Ok(Self::Establish {
                        uuid,
                        conn_id,
                        initial_reply: Some((b, from)),
                    })
                } else {
                    Ok(Self::Establish {
                        uuid,
                        conn_id,
                        initial_reply: None,
                    })
                }
            }
            MessageFlags::Data {
                has_conn_id,
                has_addr,
                addr_in_ipv6,
                has_nonce,
            } => {
                let conn_id = if has_conn_id {
                    Some(parse_conn_id(&mut b)?)
                } else {
                    None
                };

                let addr = if has_addr {
                    Some(parse_socket_addr(&mut b, !addr_in_ipv6)?)
                } else {
                    None
                };

                let nonce = if has_nonce {
                    check_remaining!(b, 2, "Enc nonce");
                    Some(b.get_u16())
                } else {
                    None
                };

                Ok(Self::Data {
                    conn_id,
                    addr,
                    payload: b.into(),
                    enc_nonce: nonce,
                })
            }
        }
    }

    pub fn to_writer(&self, w: &mut impl Write) -> anyhow::Result<()> {
        match self {
            Message::Connect {
                uuid,
                initial_data,
                dst,
                initial_data_nonce,
            } => {
                w.write_u8(
                    MessageFlags::Connect {
                        is_ipv6: matches!(dst, SocketAddr::V6(_)),
                        has_nonce: initial_data_nonce.is_some(),
                    }
                    .into(),
                )?;
                write_uuid(w, uuid.as_ref())?;
                write_socket_addr(w, dst)?;
                if let Some(nonce) = initial_data_nonce {
                    w.write_u16::<BigEndian>(*nonce)?;
                }
                w.write_all(initial_data.as_ref())?;
            }
            Message::Establish {
                uuid,
                conn_id,
                initial_reply,
            } => {
                w.write_u8(
                    MessageFlags::Establish {
                        has_reply: initial_reply.is_some(),
                        reply_in_ipv6: matches!(initial_reply, Some((_, SocketAddr::V6(_)))),
                    }
                    .into(),
                )?;
                write_uuid(w, uuid)?;
                w.write_u16::<BigEndian>(*conn_id)?;
                if let Some((data, addr)) = initial_reply {
                    write_socket_addr(w, addr)?;
                    w.write_all(data.as_ref())?;
                }
            }
            Message::Data {
                conn_id,
                addr,
                payload,
                enc_nonce,
            } => {
                w.write_u8(
                    MessageFlags::Data {
                        has_conn_id: conn_id.is_some(),
                        has_addr: addr.is_some(),
                        addr_in_ipv6: matches!(addr, Some(SocketAddr::V6(_))),
                        has_nonce: enc_nonce.is_some(),
                    }
                    .into(),
                )?;

                if let Some(c) = conn_id {
                    w.write_u16::<BigEndian>(*c)?;
                }

                if let Some(addr) = addr {
                    write_socket_addr(w, addr)?;
                }

                if let Some(nonce) = enc_nonce {
                    w.write_u16::<BigEndian>(*nonce)?;
                }

                w.write_all(payload.as_ref())?;
            }
        }

        Ok(())
    }

    pub fn to_bytes(&self) -> anyhow::Result<Bytes> {
        let mut buf = vec![];
        self.to_writer(&mut buf)?;
        Ok(buf.into())
    }
}

pub fn new_message_sink_stream(
    sink: impl Sink<Bytes, Error = anyhow::Error> + Send,
    stream: impl Stream<Item = anyhow::Result<Bytes>> + Send,
) -> (
    impl Sink<Message<'static>, Error = anyhow::Error> + Unpin + Send,
    impl Stream<Item = anyhow::Result<Message<'static>>> + Unpin + Send,
) {
    (
        Box::pin(sink.with(|m: Message| async move {
            let mut buf = vec![];
            m.to_writer(&mut buf)?;
            Ok(buf.into())
        })),
        Box::pin(stream.map(|m| m.and_then(Message::parse))),
    )
}

#[allow(dead_code)]
fn secret_key() -> &'static SecretKey {
    lazy_static! {
        static ref KEY: SecretKey =
            SecretKey::from_slice(b"=DW5<W;FJ;nPMA`&6cCpzm7jJNp3`J4a").unwrap();
    }
    &KEY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(message: Message) {
        let mut out = vec![];
        message
            .to_writer(&mut out)
            .with_context(|| format!("Writing {message:?}"))
            .unwrap();
        assert_eq!(
            message,
            Message::parse(out.into())
                .with_context(|| format!("Parsing expecting message {message:?}"))
                .unwrap()
        );
    }

    #[test]
    fn encoding_works() {
        test_message(Message::Connect {
            uuid: Bytes::from_static(b"uuid").into(),
            initial_data: Bytes::from_static(b"hello, world").into(),
            initial_data_nonce: Some(12),
            dst: "1.2.3.4:90".parse().unwrap(),
        });

        test_message(Message::Connect {
            uuid: Bytes::from_static(b"uuid").into(),
            initial_data: Bytes::from_static(b"hello, world").into(),
            initial_data_nonce: None,
            dst: "[::1]:90".parse().unwrap(),
        });

        test_message(Message::Establish {
            uuid: Bytes::from_static(b"uuid"),
            conn_id: 123,
            initial_reply: None,
        });

        test_message(Message::Establish {
            uuid: Bytes::from_static(b"uuid"),
            conn_id: 123,
            initial_reply: Some((Bytes::from_static(b"reply"), "1.2.3.4:80".parse().unwrap())),
        });

        test_message(Message::Establish {
            uuid: Bytes::from_static(b"uuid"),
            conn_id: 123,
            initial_reply: Some((Bytes::from_static(b"reply"), "[::1]:234".parse().unwrap())),
        });

        test_message(Message::Data {
            conn_id: Some(456),
            payload: Bytes::from_static(b"hello, world").into(),
            enc_nonce: Some(12),
            addr: None,
        });

        test_message(Message::Data {
            conn_id: Some(456),
            payload: Bytes::from_static(b"hello, world").into(),
            enc_nonce: Some(12),
            addr: Some("1.2.3.4:80".parse().unwrap()),
        });

        test_message(Message::Data {
            conn_id: Some(456),
            payload: Bytes::from_static(b"hello, world").into(),
            addr: Some("[::1]:80".parse().unwrap()),
            enc_nonce: None,
        });

        test_message(Message::Data {
            conn_id: None,
            payload: Bytes::from_static(b"hello, world").into(),
            addr: None,
            enc_nonce: None,
        });

        test_message(Message::Data {
            conn_id: None,
            payload: Bytes::from_static(b"hello, world").into(),
            addr: Some("1.2.3.4:80".parse().unwrap()),
            enc_nonce: Some(12),
        });

        test_message(Message::Data {
            conn_id: None,
            payload: Bytes::from_static(b"hello, world").into(),
            addr: Some("[::1]:80".parse().unwrap()),
            enc_nonce: Some(12),
        });
    }
}
