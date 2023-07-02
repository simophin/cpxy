use super::super::{ProtocolAcceptedState, ProtocolAcceptor, ProxyRequest};
use anyhow::{bail, Context};
use async_trait::async_trait;
use socks5_impl::protocol::{
    Address, Command, HandshakeMethod, HandshakeRequest, HandshakeResponse, Reply, Request,
    Response,
};
use tokio::net::TcpStream;
use crate::protocol::ProtocolReply;

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
        let req = HandshakeRequest::from_stream(&mut stream)
            .await
            .context("Reading handshake request")?;

        if !req.methods.contains(&HandshakeMethod::None) {
            bail!("Only non-password auth is supported")
        }

        HandshakeResponse::new(HandshakeMethod::None)
            .write_to(&mut stream)
            .await
            .context("Writing handshake response")?;

        let Request { command, address } = Request::from_stream(&mut stream)
            .await
            .context("Reading socks5 request")?;

        match command {
            Command::Connect => {}
            _ => {
                Response::new(Reply::CommandNotSupported, Address::unspecified())
                    .write_to(&mut stream)
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

    async fn reply(mut self, reply: ProtocolReply) -> anyhow::Result<Self::ServerStream> {
        if reply {
            Response::new(
                Reply::Succeeded,
                self.stream
                    .local_addr()
                    .context("Getting local address")?
                    .into(),
            )
        } else {
            Response::new(Reply::GeneralFailure, Address::unspecified())
        }
        .write_to(&mut self.stream)
        .await
        .context("Writing response")?;

        Ok(self.stream)
    }
}
