use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use url::Url;

pub fn is_tcp(url: &Url) -> bool {
    return url.scheme().eq_ignore_ascii_case("tcp") && url.has_host() && url.port().is_some();
}

pub async fn tcp_fetcher(
    url: Url,
) -> anyhow::Result<(SocketAddr, impl AsyncRead + AsyncWrite + Unpin)> {
    assert!(is_tcp(&url));
    let target = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
    log::info!("Connecting to tcp://{target}");
    let mut stream = TcpStream::connect(target).await?;
    let bound_address = stream.local_addr()?;
    Ok((bound_address, stream))
}
