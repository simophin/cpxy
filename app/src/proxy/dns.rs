use std::collections::{HashMap, HashSet};

use super::ProxyResult;
use anyhow::Context;
use futures_lite::{AsyncRead, AsyncWrite};
use smol::{net::resolve, spawn};

use crate::utils::write_bincode_lengthed_async;

pub async fn resolve_domains(
    domains: Vec<String>,
    mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
) -> anyhow::Result<()> {
    let domains: HashSet<String> = domains.into_iter().collect();
    let resolving_tasks = domains
        .into_iter()
        .map(|name| {
            spawn(async move {
                log::info!("Resolving domain {name}");
                let addresses = match resolve(&name).await {
                    Ok(v) => v.into_iter().map(|a| a.ip()).collect(),
                    Err(e) => {
                        log::error!("Error solving {name}: {e:?}");
                        Default::default()
                    }
                };
                (name, addresses)
            })
        })
        .collect::<Vec<_>>();

    let mut result = HashMap::with_capacity(resolving_tasks.len());
    for task in resolving_tasks {
        let (name, addresses) = task.await;
        result.insert(name, addresses);
    }

    write_bincode_lengthed_async(&mut stream, &ProxyResult::DNSResolved { addresses: result })
        .await
        .context("Writing result to client")
}
