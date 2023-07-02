use super::direct;
use super::http;
use super::socks5;
use super::stream::ProtocolStream;
use crate::protocol::{tcpman, BoxProtocolReporter, Protocol, ProxyRequest};
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

    #[serde(rename = "tcpman")]
    Tcpman(tcpman::Tcpman),
}

#[async_trait]
impl Protocol for DynamicProtocol {
    type ClientStream = ProtocolStream;

    async fn new_stream(
        &self,
        req: &ProxyRequest,
        reporter: &BoxProtocolReporter,
        fwmark: Option<u32>,
    ) -> anyhow::Result<Self::ClientStream> {
        match self {
            DynamicProtocol::Direct(s) => s
                .new_stream(req, reporter, fwmark)
                .await
                .map(ProtocolStream::Direct),

            DynamicProtocol::Http(s) => s
                .new_stream(req, reporter, fwmark)
                .await
                .map(ProtocolStream::Http),

            DynamicProtocol::Socks5(s) => s
                .new_stream(req, reporter, fwmark)
                .await
                .map(ProtocolStream::Socks5),

            DynamicProtocol::Tcpman(s) => s
                .new_stream(req, reporter, fwmark)
                .await
                .map(ProtocolStream::Tcpman),
        }
    }
}
