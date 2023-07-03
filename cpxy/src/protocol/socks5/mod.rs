pub mod server;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::{Protocol, ProxyRequest};
use crate::io::time_future;
use crate::io::{connect_tcp_marked, CounterStream};

use crate::addr::Address;
use crate::protocol::ProtocolReporter;
use socks5_impl::protocol as s5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Socks5 {
    pub address: Address,
}

#[async_trait]
impl Protocol for Socks5 {
    type ClientStream = CounterStream<TcpStream>;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &Arc<ProtocolReporter>,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        let (mut upstream, delay) = time_future(connect_tcp_marked(&self.address, fwmark))
            .await
            .context("Connecting to SOCKS sever")?;

        reporter.report_delay(delay);

        s5::HandshakeRequest::new(vec![s5::HandshakeMethod::None])
            .write_to(&mut upstream)
            .await
            .context("Writing handshake request")?;

        let res = s5::HandshakeResponse::from_stream(&mut upstream)
            .await
            .context("Receiving handshake response")?;

        if res.method != s5::HandshakeMethod::None {
            bail!("Unsupported handshake method");
        }

        s5::Request::new(s5::Command::Connect, req.dst.clone().into())
            .write_to(&mut upstream)
            .await
            .context("Writing request")?;

        let res = s5::Response::from_stream(&mut upstream)
            .await
            .context("Receiving response")?;

        if res.reply != s5::Reply::Succeeded {
            bail!("Error in SOCKS server: {:?}", res.reply);
        }

        let mut upstream = CounterStream::new(upstream, reporter.clone());
        match &req.initial_data {
            Some(b) if b.len() > 0 => upstream
                .write_all(b)
                .await
                .context("Writing initial data")?,
            _ => {}
        }

        Ok(upstream)
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::test;

    #[tokio::test]
    async fn socks5_proxy_works() {
        let _ = env_logger::try_init();

        test::test_protocol_valid_config(
            |addr| super::Socks5 {
                address: addr.into(),
            },
            Some(super::server::Socks5Acceptor::default()),
        )
        .await
        .expect("socks5 proxy works");
    }
}
