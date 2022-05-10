mod cipher;
mod dgram;
mod proto;
pub mod server;
mod udp_stream;

use std::borrow::Cow;

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite};
use serde::{Deserialize, Serialize};

use crate::io::{connect_tcp_marked, AsyncStreamCounter};
use crate::{socks5::Address, url::HttpUrl};

use self::{
    cipher::strategy::EncryptionStrategy,
    dgram::{create_udp_sink, create_udp_stream},
};

use super::{AsyncStream, BoxedSink, BoxedStream, Protocol, Stats, TrafficType};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TcpMan {
    pub address: Address<'static>,
    pub ssl: bool,
    pub allows_udp: bool,
}

impl TcpMan {
    async fn send_request<'a>(
        &self,
        dst: &Address<'_>,
        req: proto::Request<'a>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<impl AsyncRead + AsyncWrite + Unpin + Send + Sync> {
        let stream = connect_tcp_marked(&self.address, fwmark)
            .await
            .context("Connect to TCPMan server")?;

        let initial_data = req.to_vec();

        cipher::client::connect(
            &HttpUrl {
                is_https: self.ssl,
                address: self.address.clone(),
                path: Cow::Borrowed("/"),
            },
            AsyncStreamCounter::new(stream, stats.rx.clone(), stats.tx.clone()),
            EncryptionStrategy::new_send(true, dst.get_port(), self.ssl),
            EncryptionStrategy::new_receive(true, dst.get_port()),
            initial_data,
        )
        .await
    }
}

#[async_trait]
impl Protocol for TcpMan {
    fn supports(&self, t: TrafficType) -> bool {
        match (t, self.allows_udp) {
            (TrafficType::Datagram, false) => false,
            _ => true,
        }
    }

    async fn new_stream(
        &self,
        dst: &Address<'_>,
        initial_data: Option<&[u8]>,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Box<dyn AsyncStream>> {
        Ok(Box::new(
            self.send_request(
                dst,
                proto::Request::TCP {
                    dst: dst.clone(),
                    initial_data: initial_data.unwrap_or_default(),
                },
                stats,
                fwmark,
            )
            .await?,
        ))
    }

    async fn new_datagram(
        &self,
        dst: &Address<'_>,
        initial_data: Bytes,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<(BoxedSink, BoxedStream)> {
        let (r, w) = self
            .send_request(
                dst,
                proto::Request::UDP {
                    dst: dst.clone(),
                    initial_data: initial_data.as_ref(),
                },
                stats,
                fwmark,
            )
            .await?
            .split();
        Ok((
            Box::pin(create_udp_sink(w)),
            Box::pin(create_udp_stream(r, None)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::test::*,
        rt::{block_on, spawn},
        test::create_tcp_server,
    };

    #[test]
    fn tcpman_works() {
        std::env::set_var("RUST_LOG", "debug");
        let _ = env_logger::try_init();
        block_on(async move {
            let (server, addr) = create_tcp_server().await;
            let _task = spawn(super::server::run_server(server));

            let p = TcpMan {
                address: addr.into(),
                ssl: false,
                allows_udp: true,
            };

            test_protocol_http(&p).await;
            test_protocol_tcp(&p).await;
            test_protocol_udp(&p).await;
        });
    }
}
