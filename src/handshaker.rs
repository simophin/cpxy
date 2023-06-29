use crate::io::read_data_with_timeout;
use crate::protocol::http::server::extract_proxy_request;
use crate::protocol::ProxyRequest;
use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NO_PASSWORD,
};
use anyhow::{bail, Context};
use bytes::BytesMut;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

enum ConnectionType {
    Socks5,
    Http,
}

pub struct Handshaker<S> {
    stream: S,
    conn_type: ConnectionType,
}

impl<S: AsyncBufRead + AsyncWrite + Unpin> Handshaker<S> {
    pub async fn start(mut stream: S) -> anyhow::Result<(Self, ProxyRequest)> {
        let buf = stream.fill_buf().await.context("Reading buffer")?;
        if buf.len() == 0 {
            bail!("EOF while reading request");
        }

        return match buf[0] {
            0x5 => Self::start_socks5(stream).await,
            0x4 => bail!("Unsupported protocol: socks4"),
            _ => Self::start_http(stream).await,
        };
    }

    async fn start_socks5(mut stream: S) -> anyhow::Result<(Self, ProxyRequest)> {
        log::debug!("Handshaking socks5");
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

        let initial_data = read_data_with_timeout(&mut stream).await?;

        Ok((
            Self {
                stream,
                conn_type: ConnectionType::Socks5,
            },
            ProxyRequest {
                dst: req.address,
                initial_data,
            },
        ))
    }

    async fn start_http(mut stream: S) -> anyhow::Result<(Self, ProxyRequest)> {
        let request = extract_proxy_request(&mut stream).await?;

        Ok((
            Self {
                stream,
                conn_type: ConnectionType::Http,
            },
            request,
        ))
    }

    pub async fn respond_ok(mut self) -> anyhow::Result<S> {
        if let ConnectionType::Socks5 = self.conn_type {
            ClientConnRequest::respond(
                &mut self.stream,
                ConnStatusCode::GRANTED,
                &Address::default(),
            )
            .await?;
        }

        Ok(self.stream)
    }

    pub async fn respond_err(mut self, err: &anyhow::Error) -> anyhow::Result<()> {
        let body = format!("{err:?}");
        self.stream
            .write_all(
                format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await?;

        Ok(())
    }
}
