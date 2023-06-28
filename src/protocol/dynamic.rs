use super::direct;
use super::http;
use super::socks5;
use super::stream::ProtocolStream;
use crate::protocol::{Protocol, ProxyRequest, Stats};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
#[serde(tag = "type")]
pub enum DynamicProtocol {
    #[serde(rename = "direct")]
    Direct(direct::Direct),

    #[serde(rename = "http")]
    Http(http::HttpProxy),

    #[serde(rename = "socks5")]
    Socks5(socks5::Socks5),
}

#[async_trait]
impl Protocol for DynamicProtocol {
    type Stream = ProtocolStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        stats: &Stats,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::Stream> {
        match self {
            DynamicProtocol::Direct(s) => s
                .new_stream(req, stats, fwmark)
                .await
                .map(ProtocolStream::Direct),
            DynamicProtocol::Http(s) => s
                .new_stream(req, stats, fwmark)
                .await
                .map(ProtocolStream::Http),

            DynamicProtocol::Socks5(s) => s
                .new_stream(req, stats, fwmark)
                .await
                .map(ProtocolStream::Socks5),
        }
    }
}
