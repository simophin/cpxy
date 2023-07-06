use std::{
    borrow::Cow,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod client;
pub mod list_base;
pub mod server;

#[derive(Serialize, Deserialize, Debug)]
enum Request<'a> {
    Recursive(Cow<'a, str>),
}

#[derive(Serialize, Deserialize, Debug)]
enum Response<'a> {
    RecursiveResult(Cow<'a, [IpAddr]>),
    Err(Cow<'a, str>),
}

#[async_trait]
pub trait DomainListProvider {
    async fn dns_server_for_domain(&self, domain: &str) -> Option<SocketAddr>;
}

#[async_trait]
impl<P: DomainListProvider + Send + Sync> DomainListProvider for Arc<P> {
    async fn dns_server_for_domain(&self, domain: &str) -> Option<SocketAddr> {
        (**self).dns_server_for_domain(domain).await
    }
}

#[cfg(test)]
mod tests {
    use async_shutdown::Shutdown;
    use orion::aead;
    use tokio::{net::UdpSocket, spawn};

    use super::*;

    #[derive(Clone)]
    struct EmptyDomainListProvider;

    #[async_trait]
    impl DomainListProvider for EmptyDomainListProvider {
        async fn dns_server_for_domain(&self, _domain: &str) -> Option<SocketAddr> {
            None
        }
    }

    #[tokio::test]
    async fn dns_server_works() {
        let shutdown = Shutdown::new();
        let server_socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("To bind a udp socket");
        let addr = server_socket.local_addr().expect("To have a local address");
        let key = aead::SecretKey::default();

        spawn(server::run_server(
            shutdown.clone(),
            server::Settings {
                key: aead::SecretKey::from_slice(key.unprotected_as_bytes())
                    .expect("To clone a key"),
            },
            server_socket,
            EmptyDomainListProvider {},
        ));

        let records = client::fetch(&client::Settings { addr, key }, "www.baidu.com")
            .await
            .expect("To fetch dns");

        println!("Got DNS records: {records:#?}");

        assert!(!records.is_empty());
    }
}
