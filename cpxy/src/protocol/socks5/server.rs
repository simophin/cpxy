use anyhow::{bail, Context};
use async_trait::async_trait;
use bytes::Bytes;
use socks5_impl::protocol::{
    handshake::Request as HandshakeRequest, handshake::Response as HandshakeResponse, Address,
    AsyncStreamOperation, AuthMethod, Command, Reply, Request, Response,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::super::{ProtocolAcceptedState, ProtocolAcceptor, ProxyRequest};

#[derive(Default, Clone)]
pub struct Socks5Acceptor;

pub struct Socks5AcceptedState {
    stream: TcpStream,
}

#[async_trait]
impl ProtocolAcceptor for Socks5Acceptor {
    type AcceptedState = Socks5AcceptedState;

    async fn accept(
        &self,
        mut stream: TcpStream,
    ) -> anyhow::Result<(Self::AcceptedState, ProxyRequest)> {
        let req = HandshakeRequest::retrieve_from_async_stream(&mut stream)
            .await
            .context("Reading handshake request")?;

        if !req.evaluate_method(AuthMethod::NoAuth) {
            bail!("Only non-password auth is supported")
        }

        HandshakeResponse::new(AuthMethod::NoAuth)
            .write_to_async_stream(&mut stream)
            .await
            .context("Writing handshake response")?;

        let Request { command, address } = Request::retrieve_from_async_stream(&mut stream)
            .await
            .context("Reading socks5 request")?;

        match command {
            Command::Connect => {}
            _ => {
                Response::new(Reply::CommandNotSupported, Address::unspecified())
                    .write_to_async_stream(&mut stream)
                    .await
                    .context("Writing response")?;

                bail!("Unsupported command")
            }
        }

        Ok((
            Socks5AcceptedState { stream },
            crate::addr::Address::from(address).into(),
        ))
    }
}

#[async_trait]
impl ProtocolAcceptedState for Socks5AcceptedState {
    type ServerStream = TcpStream;

    async fn reply_success(
        mut self,
        initial_data: Option<Bytes>,
    ) -> anyhow::Result<Self::ServerStream> {
        Response::new(
            Reply::Succeeded,
            self.stream
                .local_addr()
                .context("Getting local address")?
                .into(),
        )
        .write_to_async_stream(&mut self.stream)
        .await?;

        if let Some(data) = initial_data {
            self.stream
                .write_all(&data)
                .await
                .context("Writing initial data")?;
        }

        Ok(self.stream)
    }

    async fn reply_error(
        mut self,
        _error: Option<impl AsRef<str> + Send + Sync>,
    ) -> anyhow::Result<()> {
        Response::new(Reply::GeneralFailure, Address::unspecified())
            .write_to_async_stream(&mut self.stream)
            .await?;
        Ok(())
    }
}
