use super::ClientConnRequest;
use crate::parse::ParseError;
use crate::socks5::greeting::{ClientGreeting, AUTH_NOT_ACCEPTED, AUTH_NO_PASSWORD};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

pub async fn negotiate_request(
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<ClientConnRequest> {
    let mut buf = vec![0u8; 1024];

    let greeting = {
        let mut n = 0;
        loop {
            match rx.read(&mut buf.as_mut_slice()[n..]).await? {
                0 => {
                    return Err(ParseError::unexpected(
                        "greeting",
                        "not enough bytes",
                        "enough bytes",
                    )
                    .into());
                }
                v => n += v,
            };

            if let Some(g) = ClientGreeting::parse(&buf.as_slice()[..n])? {
                break g;
            }
        }
    };

    log::info!("Got {:?}", greeting);

    if !greeting.auths.contains(&AUTH_NO_PASSWORD) {
        ClientGreeting::respond(AUTH_NOT_ACCEPTED, tx).await?;
        return Err(
            ParseError::unexpected("auth type", format!("{:?}", greeting.auths), "0x00").into(),
        );
    }

    ClientGreeting::respond(AUTH_NO_PASSWORD, tx).await?;

    let req = {
        let mut n = 0;
        loop {
            match rx.read(&mut buf.as_mut_slice()[n..]).await? {
                0 => {
                    return Err(ParseError::unexpected(
                        "client req",
                        "not enough bytes",
                        "enough bytes",
                    )
                    .into());
                }
                v => n += v,
            }

            if let Some(v) = ClientConnRequest::parse(&buf.as_slice()[..n])? {
                break v;
            }
        }
    };

    log::info!("Got {:?}", req);
    Ok(req)
}
