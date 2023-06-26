use crate::http::{httparse_to_hyper_request, parse_request};
use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NO_PASSWORD,
};
use anyhow::{bail, Context};
use hyper::http;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite};

enum ConnectionType {
    Socks5,
    Socks4,
    Http,
    HttpTunnel,
    Direct,
}

pub struct Handshaker<S> {
    stream: S,
    conn_type: ConnectionType,
}

pub enum Request {
    Tcp(Address<'static>),
    Http(http::Request<()>),
}

impl<S: AsyncBufRead + AsyncWrite + Unpin> Handshaker<S> {
    pub async fn start(mut stream: S) -> anyhow::Result<(Self, Request)> {
        let buf = stream.fill_buf().await.context("Reading buffer")?;
        if buf.len() == 0 {
            bail!("EOF while reading request");
        }

        return match buf[0] {
            0x5 => {
                // Socks5
                log::debug!("Incoming traffic is SOCKS5");
                Self::start_socks5(stream).await
            }

            0x4 => {
                // Socks4
                log::debug!("Incoming traffic is SOCKS4");
                Self::start_socks4(stream).await
            }

            _ => {
                // Assuming http
                log::debug!("Incoming traffic is treated as HTTP");
                Self::start_http(stream).await
            }
        };
    }

    async fn start_socks5(mut stream: S) -> anyhow::Result<(Self, Request)> {
        let ClientGreeting { auths } = ClientGreeting::parse(&mut stream).await?;

        if !auths.contains(&AUTH_NO_PASSWORD) {
            bail!("No supported auth methods");
        }

        ClientGreeting::respond(AUTH_NO_PASSWORD, &mut stream).await?;

        let req = ClientConnRequest::parse_async(&mut stream).await?;

        if req.cmd != Command::CONNECT_TCP {
            let _ = ClientConnRequest::respond(
                &mut stream,
                ConnStatusCode::UNSUPPORTED_COMMAND,
                &Address::default(),
            )
            .await;
            bail!("Unsupported command");
        }

        Ok((
            Self {
                stream,
                conn_type: ConnectionType::Socks5,
            },
            Request::Tcp(req.address),
        ))
    }

    async fn start_socks4(mut stream: S) -> anyhow::Result<(Self, Request)> {
        bail!("Sock4 unsupported")
    }

    async fn start_http(mut stream: S) -> anyhow::Result<(Self, Request)> {
        let request = parse_request(&mut stream, httparse_to_hyper_request).await?;
        match request.method() {
            hyper::Method::CONNECT =>
        }

        Ok((
            Self {
                stream,
                conn_type: ConnectionType::Http,
            },
            Request::Http(request),
        ))
    }

    pub async fn respond_ok(self) -> anyhow::Result<S> {
        todo!()
    }

    pub async fn respond_err(self, err: &anyhow::Error) {
        todo!()
    }
}
