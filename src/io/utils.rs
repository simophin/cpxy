use std::{borrow::Cow, net::SocketAddr, sync::Arc};

use crate::rt::{net::UdpSocket, spawn};
use futures_lite::{future::race, io::split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    buf::{Buf, RWBuffer},
    socks5::Address,
};

use super::send_to_addr;

pub async fn copy_udp_and_udp(
    src: UdpSocket,
    mut src_buf: Buf,
    dst: UdpSocket,
    src_hdr_len: Option<usize>,
    dst_hdr_len: Option<usize>,
    src_to_dst_fn: impl Fn(SocketAddr, &mut Buf) -> anyhow::Result<Option<(Address, Cow<[u8]>)>>
        + Send
        + Sync
        + 'static,
    dst_to_src_fn: impl Fn(SocketAddr, &mut Buf) -> anyhow::Result<Option<(Address, Cow<[u8]>)>>
        + Send
        + Sync
        + 'static,
) -> anyhow::Result<()> {
    let src = Arc::new(src);
    let dst = Arc::new(dst);

    let task1 = {
        let src = src.clone();
        let dst = dst.clone();
        spawn(async move {
            let start = dst_hdr_len.unwrap_or(0);
            if start >= src_buf.capacity() {
                panic!("Header size is greater than buf capacity");
            }

            loop {
                src_buf.set_len(src_buf.capacity());
                let addr = match src.recv_from(&mut src_buf[start..]).await? {
                    (n, a) if n > 0 => {
                        src_buf.set_len(start + n);
                        a
                    }
                    _ => return Ok(()),
                };

                if let Some((addr, buf)) = src_to_dst_fn(addr, &mut src_buf)? {
                    send_to_addr(&dst, buf.as_ref(), &addr).await?;
                }
            }
        })
    };

    let task2 = {
        let src = src.clone();
        let dst = dst.clone();
        spawn(async move {
            let mut buf = Buf::new_with_len(65536, 65536);
            let start = src_hdr_len.unwrap_or(0);
            if start >= buf.capacity() {
                panic!("Header size is greater than buf capacity");
            }

            loop {
                buf.set_len(buf.capacity());
                let addr = match dst.recv_from(&mut buf[start..]).await? {
                    v if v.0 > 0 => {
                        buf.set_len(start + v.0);
                        v.1
                    }
                    _ => return Ok(()),
                };

                if let Some((addr, buf)) = dst_to_src_fn(addr, &mut buf)? {
                    send_to_addr(&src, buf.as_ref(), &addr).await?;
                }
            }
        })
    };

    race(task1, task2).await
}

pub async fn copy_udp_and_stream(
    udp: UdpSocket,
    mut udp_buf: Buf,
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    stream_hdr_max_size: Option<usize>,
    transform_udp_buf: impl Fn(SocketAddr, Option<&mut Buf>, &mut Buf) -> anyhow::Result<()>
        + Send
        + Sync
        + 'static,
    transform_stream_buf: impl Fn(&[u8], &mut Buf) -> anyhow::Result<Option<(usize, Address<'static>)>>
        + Send
        + Sync
        + 'static,
) -> anyhow::Result<()> {
    let udp = Arc::new(udp);
    let (mut r, mut w) = split(stream);

    let task1 = {
        let udp = udp.clone();
        spawn(async move {
            let mut stream_hdr = stream_hdr_max_size.map(Buf::new);
            loop {
                udp_buf.set_len(udp_buf.capacity());

                let addr = match udp.recv_from(&mut udp_buf).await? {
                    v if v.0 == 0 => return Ok(()),
                    (n, addr) => {
                        udp_buf.set_len(n);
                        addr
                    }
                };

                transform_udp_buf(addr, stream_hdr.as_mut(), &mut udp_buf)?;
                match stream_hdr.as_ref() {
                    Some(hdr) if !hdr.is_empty() => {
                        w.write_all(&hdr).await?;
                    }
                    _ => {}
                };

                if !udp_buf.is_empty() {
                    w.write_all(&udp_buf).await?;
                }
            }
        })
    };

    let task2 = {
        let udp = udp.clone();
        spawn(async move {
            loop {
                let mut stream_buf = RWBuffer::new(67000, 67000);
                let mut udp_buf = Buf::new(65536);
                match r.read(stream_buf.write_buf()).await? {
                    0 => return Ok(()),
                    v => stream_buf.advance_write(v),
                };

                while stream_buf.remaining_read() > 0 {
                    udp_buf.set_len(0);
                    match transform_stream_buf(stream_buf.read_buf(), &mut udp_buf)? {
                        Some((offset, addr)) => {
                            send_to_addr(&udp, &udp_buf, &addr).await?;
                            stream_buf.advance_read(offset);
                        }
                        None => break,
                    }
                }

                if stream_buf.should_compact() {
                    stream_buf.compact();
                }
            }
        })
    };

    race(task1, task2).await
}
