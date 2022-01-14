use super::ClientConnRequest;
use crate::parse::Parsable;
use crate::socks5::req::{ClientGreeting, AUTH_NO_PASSWORD};
use anyhow::anyhow;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::io::ErrorKind::UnexpectedEof;

pub async fn negotiate_request(
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<ClientConnRequest<'static>> {
    let mut buf = vec![0u8; 1024];

    // Wait for greeting...
    {
        let mut n = 0;
        loop {
            match rx.read(&mut buf.as_mut_slice()[n..]).await? {
                0 => return Err(std::io::Error::from(UnexpectedEof).into()),
                v => n += v,
            }

            match ClientGreeting::parse(&buf[..n])? {
                None => continue,
                Some((_, g)) if !g.auths.contains(&AUTH_NO_PASSWORD) => {
                    tx.write_all(&[0x5, 0xFF]).await?;
                    return Err(anyhow!("No supported auth method"));
                }
                _ => {
                    tx.write_all(&[0x5, AUTH_NO_PASSWORD]).await?;
                    break;
                }
            }
        }
    }

    // Wait for client request
    {
        let mut n = 0;
        loop {
            match rx.read(&mut buf.as_mut_slice()[n..]).await? {
                0 => return Err(std::io::Error::from(UnexpectedEof).into()),
                v => n += v,
            }

            match ClientConnRequest::parse(&buf[..n])? {
                None => continue,
                Some((_, g)) => {
                    return Ok(g.to_owned());
                }
            }
        }
    }
}
