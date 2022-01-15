use crate::parse::{Parsable, ParseError, ParseResult, Writable};
use crate::socks5::{Address, UdpPacket};
use anyhow::anyhow;
use async_std::net::UdpSocket;
use bytes::{Buf, BufMut};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::borrow::Cow;
use std::fmt::Debug;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum UdpFrame<'a> {
    Send(Cow<'a, [u8]>),
    SendTo(Address, Cow<'a, [u8]>),
}

impl<'a> UdpFrame<'a> {
    fn parse<'buf>(mut buf: &'buf [u8]) -> ParseResult<Self>
    where
        'buf: 'a,
    {
        if !buf.has_remaining() {
            return Ok(None);
        }

        match buf.get_u8() {
            0 => {
                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let pkt_len = buf.get_u16() as usize;
                if buf.remaining() < pkt_len {
                    return Ok(None);
                }

                let (pkt, _) = buf.split_at(pkt_len);
                Ok(Some((1 + 2 + pkt_len, Self::Send(Cow::Borrowed(pkt)))))
            }

            1 => {
                let (offset, addr) = match Address::parse(buf)? {
                    Some(a) => a,
                    None => return Ok(None),
                };

                buf.advance(offset);

                if buf.remaining() < 2 {
                    return Ok(None);
                }

                let pkt_len = buf.get_u16() as usize;
                if buf.remaining() < pkt_len {
                    return Ok(None);
                }

                let (pkt, _) = buf.split_at(pkt_len);
                Ok(Some((
                    1 + offset + 2 + pkt_len,
                    Self::SendTo(addr, Cow::Borrowed(pkt)),
                )))
            }

            v => Err(ParseError::unexpected("Message type", v, "0 or 1")),
        }
    }

    async fn write_async(&self, w: &mut (impl AsyncWrite + Unpin + ?Sized)) -> anyhow::Result<()> {
        match self {
            Self::Send(data) => {
                if data.len() > u16::MAX as usize {
                    return Err(anyhow!("Exceeded max data len"));
                }

                let mut hdr = [0u8, 0, 0];
                (&mut hdr[1..]).put_u16(data.len() as u16);

                w.write_all(&hdr).await?;
                w.write_all(data.as_ref()).await?;
                Ok(())
            }

            Self::SendTo(addr, data) => {
                if data.len() > u16::MAX as usize {
                    return Err(anyhow!("Exceeded max data len"));
                }

                let mut hdr = Vec::with_capacity(3 + addr.write_len());
                hdr.put_u8(1u8);
                assert!(addr.write(&mut hdr));
                hdr.put_u16(data.len() as u16);

                w.write_all(&hdr).await?;
                w.write_all(data.as_ref()).await?;
                Ok(())
            }
        }
    }
}

pub async fn copy_udp_to_frame(
    src: &UdpSocket,
    target: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];
    loop {
        let (n, peer) = match src.recv_from(buf.as_mut_slice()).await {
            Ok(v) if v.0 > 0 => v,
            Ok(_) => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        let frame = match src.peer_addr().as_ref() {
            Ok(peer_addr) if peer_addr == &peer => {
                UdpFrame::Send(Cow::Borrowed(&buf.as_slice()[..n]))
            }
            _ => UdpFrame::SendTo(Address::IP(peer), Cow::Borrowed(&buf.as_slice()[..n])),
        };

        frame.write_async(target).await?;
    }
}

pub async fn copy_frame_to_udp(
    src: &mut (impl AsyncRead + Unpin + ?Sized),
    target: &UdpSocket,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];
    let mut read_cursor = 0;
    let mut write_cursor = 0;
    loop {
        match src.read(&mut buf.as_mut_slice()[write_cursor..]).await? {
            0 => return Ok(()),
            v => write_cursor += v,
        };

        while let Some((offset, frame)) =
            UdpFrame::parse(&buf.as_slice()[read_cursor..write_cursor])?
        {
            read_cursor += offset;
            match frame {
                UdpFrame::Send(data) => target.send(data.as_ref()).await?,
                UdpFrame::SendTo(addr, data) => match addr {
                    Address::IP(ip) => target.send_to(data.as_ref(), ip).await?,
                    Address::Name(host, name) => {
                        target
                            .send_to(data.as_ref(), &format!("{host}:{name}"))
                            .await?
                    }
                },
            };
        }
    }
}

pub async fn copy_socks5_udp_to_frame(
    src: &UdpSocket,
    socks_requested_address: &Address,
    target: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];
    loop {
        let n = match src.recv(buf.as_mut_slice()).await? {
            0 => return Ok(()),
            v => v,
        };

        let UdpPacket {
            frag_no,
            addr,
            data,
        } = UdpPacket::parse(&buf.as_slice()[..n])?;
        if frag_no != 0 {
            log::info!("Ignoring fragmented UDP packet sending to {addr}");
            continue;
        }

        let frame = if &addr == socks_requested_address {
            UdpFrame::Send(data)
        } else {
            UdpFrame::SendTo(addr, data)
        };

        frame.write_async(target).await?;
    }
}

pub async fn copy_frame_to_socks5_udp(
    src: &mut (impl AsyncRead + Unpin + ?Sized),
    socks_requested_address: &Address,
    target: &UdpSocket,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 65536];
    let mut write_cursor = 0;
    let mut read_cursor = 0;
    let mut udp_buf = Vec::new();
    loop {
        match src.read(&mut buf.as_mut_slice()[write_cursor..]).await? {
            0 => return Ok(()),
            v => write_cursor += v,
        };

        while let Some((offset, frame)) =
            UdpFrame::parse(&buf.as_slice()[read_cursor..write_cursor])?
        {
            let packet = match frame {
                UdpFrame::Send(data) => UdpPacket {
                    frag_no: 0,
                    addr: socks_requested_address.clone(),
                    data,
                },
                UdpFrame::SendTo(addr, data) => UdpPacket {
                    frag_no: 0,
                    addr,
                    data,
                },
            };
            read_cursor += offset;

            udp_buf.clear();
            packet.write(&mut udp_buf)?;
            target.send(&udp_buf).await?;
        }

        if read_cursor != write_cursor {
            buf.copy_within(read_cursor..write_cursor, 0);
            write_cursor = read_cursor;
            read_cursor = 0;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[async_std::test]
    async fn codec_works() {
        let mut buf: Vec<u8> = Default::default();
        let data = b"hello, world";

        UdpFrame::SendTo(Default::default(), Cow::Borrowed(data))
            .write_async(&mut buf)
            .await
            .unwrap();
        UdpFrame::Send(Cow::Borrowed(data))
            .write_async(&mut buf)
            .await
            .unwrap();

        assert!(buf.len() > 0);

        {
            let mut buf = buf.as_slice();
            let (offset, msg1) = UdpFrame::parse(buf)
                .expect("To parse 1st msg")
                .expect("Message to be there");
            buf.advance(offset);
            let (offset, msg2) = UdpFrame::parse(buf)
                .expect("To parse 2nd msg")
                .expect("Message to be there");
            buf.advance(offset);
            let msg3 = UdpFrame::parse(buf).expect("To parse 3rd msg");

            assert_eq!(
                msg1,
                UdpFrame::SendTo(Default::default(), Cow::Borrowed(data))
            );
            assert_eq!(msg2, UdpFrame::Send(Cow::Borrowed(data)));
            assert_eq!(msg3, None);
        }
    }
}
