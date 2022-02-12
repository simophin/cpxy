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

#[cfg(test)]
mod test {
    use crate::{test::duplex, utils::read_bincode_lengthed_async};

    use super::*;

    #[test]
    fn test_resolve_works() {
        smol::block_on(async move {
            let (mut near, far) = duplex(1).await;
            resolve_domains(
                vec![
                    String::from("www.google.com"),
                    String::from("www.facebook.com"),
                ],
                far,
            )
            .await
            .unwrap();

            let res: ProxyResult = read_bincode_lengthed_async(&mut near).await.unwrap();
            assert!(
                matches!(res, ProxyResult::DNSResolved { addresses } if addresses.get("www.google.com").unwrap().len() > 0 && 
            addresses.get("www.facebook.com").unwrap().len() > 0)
            );
        })
    }
}
