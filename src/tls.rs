pub use f::connect_tls;

#[cfg(target_arch = "mips")]
mod f {
    use anyhow::bail;
    use futures::{AsyncRead, AsyncWrite};

    pub async fn connect_tls<T: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static>(
        host: &str,
        _: T,
    ) -> anyhow::Result<T> {
        bail!("Unsupported TLS connection to: {host}")
    }
}

#[cfg(not(target_arch = "mips"))]
mod f {
    use futures::{AsyncRead, AsyncWrite};
    use std::sync::Arc;

    use futures_rustls::{
        client::TlsStream,
        rustls::{self, OwnedTrustAnchor, RootCertStore},
        TlsConnector,
    };
    use lazy_static::lazy_static;

    fn create_config() -> rustls::ClientConfig {
        let mut root_store = RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    }

    pub async fn connect_tls<T: AsyncRead + AsyncWrite + Unpin + Send + Sync>(
        host: &str,
        stream: T,
    ) -> anyhow::Result<TlsStream<T>> {
        lazy_static! {
            static ref CONFIG: Arc<rustls::ClientConfig> = Arc::new(create_config());
        }

        Ok(TlsConnector::from(CONFIG.clone())
            .connect(host.try_into()?, stream)
            .await?)
    }
}
