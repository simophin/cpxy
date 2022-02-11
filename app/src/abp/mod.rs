use std::{
    borrow::Cow,
    io::Read,
    path::PathBuf,
    sync::RwLock,
    time::{Duration, SystemTime},
};

use crate::{fetch::fetch_http, socks5::Address};
use adblock::{
    engine::Engine,
    lists::{FilterSet, ParseOptions},
};
use anyhow::{anyhow, bail};
use chrono::DateTime;
use futures_lite::AsyncWriteExt;
use lazy_static::lazy_static;
use rust_embed::RustEmbed;
use smol::fs::File;
use url::Url;

fn create_engine(data: &[u8]) -> anyhow::Result<Engine> {
    let mut engine = Engine::new(true);
    engine
        .deserialize(data)
        .map_err(|e| anyhow!("Error init adblock engine: {e:?}"))?;
    Ok(engine)
}

struct EngineState {
    engine: Option<(Engine, SystemTime)>,
    cache_file_path: Option<PathBuf>,
}

#[derive(RustEmbed)]
#[folder = "src/abp/dat"]
struct Asset;

impl EngineState {
    fn new(file_name: &str) -> Self {
        let cache_file_path = dirs::data_dir().map(|mut r| {
            r.push("cjk_proxy");
            r.push("abp");
            r.push(file_name);
            r
        });

        let engine = cache_file_path.as_ref().and_then(|p| {
            let meta = std::fs::metadata(p).ok()?;
            let mut file = std::fs::File::open(p).ok()?;
            let mut buf = Vec::with_capacity(meta.len() as usize);
            let _ = file.read_to_end(&mut buf).ok()?;
            Some((
                create_engine(buf.as_ref()).ok()?,
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        });

        Self {
            engine,
            cache_file_path,
        }
    }

    fn from_embedded(asset_name: &str, cache_file_name: &str) -> Self {
        let mut r = Self::new(cache_file_name);
        if r.engine.is_some() {
            return r;
        }

        if let Some(f) = Asset::get(asset_name) {
            if let Ok(engine) = create_engine(f.data.as_ref()) {
                r.engine = Some((
                    engine,
                    SystemTime::UNIX_EPOCH
                        + Duration::from_secs(f.metadata.last_modified().unwrap_or_default()),
                ))
            }
        }

        r
    }
}

fn gfw_engine() -> &'static RwLock<EngineState> {
    lazy_static! {
        static ref ENGINE: RwLock<EngineState> =
            RwLock::new(EngineState::from_embedded("gfw_list.dat", "gfwlist.abp"));
    }
    &ENGINE
}

fn abp_engine() -> &'static RwLock<EngineState> {
    lazy_static! {
        static ref ENGINE: RwLock<EngineState> = RwLock::new(EngineState::new("abplist.abp"));
    }
    &ENGINE
}

async fn update_engine(
    state: &RwLock<EngineState>,
    proxy: &Address,
    rule_list_url: &str,
    is_base64: bool,
) -> anyhow::Result<usize> {
    let url = Url::parse(rule_list_url)?;
    log::info!("Downloading rule list: {url}");

    let last_modified = match state.read() {
        Ok(g) => {
            if let EngineState {
                engine: Some((_, t)),
                ..
            } = &*g
            {
                Some(t.clone())
            } else {
                None
            }
        }
        Err(_) => bail!("Error locking state"),
    };

    let last_modified = last_modified
        .map(|v| {
            Cow::Owned(
                DateTime::<chrono::Utc>::from(v)
                    .format("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string(),
            )
        })
        .into_iter()
        .map(|v| ("If-Modified-Since", v));

    let mut body = match fetch_http(rule_list_url, "GET", last_modified, proxy, None).await? {
        mut r if r.status_code == 200 => r.body().await?,
        r if r.status_code == 304 => return Ok(0),
        r => bail!("Invalid http response: {}", r.status_code),
    };

    if is_base64 {
        // Remove new lines first
        body.retain(|x| *x != b'\r' && *x != b'\n');
        body = base64::decode(body)?;
    }

    let mut filter_set = FilterSet::new(true);
    let mut line_count = 0;

    for line in body.split(|x| *x == b'\n') {
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        if line.starts_with("#") || line.starts_with("!") || line.is_empty() {
            continue;
        }

        if let Err(err) = filter_set.add_filter(line, ParseOptions::default()) {
            log::error!("Error pasing rule: '{line}': {err:?}");
        } else {
            line_count += 1;
        }
    }
    drop(body);

    let last_updated = SystemTime::now();
    let new_engine = Engine::from_filter_set(filter_set, true);

    let (file_to_write, contents) = match state.write() {
        Ok(mut g) => {
            let contents = g
                .cache_file_path
                .as_ref()
                .and_then(|_| new_engine.serialize_compressed().ok());
            g.engine = Some((new_engine, last_updated));
            (g.cache_file_path.clone(), contents)
        }
        Err(_) => bail!("Error locking engine state"),
    };

    if let (Some(p), Some(buf)) = (file_to_write, contents) {
        if let Some(parent) = p.parent() {
            smol::fs::create_dir_all(parent).await?;
        }

        let mut file = File::create(p).await?;
        file.write_all(buf.as_ref()).await?;
        file.flush().await?;
    }

    Ok(line_count)
}

fn matches_abp(state: &RwLock<EngineState>, addr: &Address) -> bool {
    let state = match state.read() {
        Ok(g) => g,
        Err(_) => return false,
    };

    let engine = match state.engine.as_ref() {
        Some((v, _)) => v,
        _ => return false,
    };

    let url = match addr.get_port() {
        443 => format!("https://{}", addr.get_host()),
        _ => format!("http://{}", addr.get_host()),
    };

    engine
        .check_network_urls(url.as_str(), url.as_str(), "")
        .matched
}

pub async fn update_gfw_list(proxy: &Address) -> anyhow::Result<usize> {
    match update_engine(
        gfw_engine(),
        proxy,
        "https://raw.githubusercontent.com/gfwlist/gfwlist/master/gfwlist.txt",
        true,
    )
    .await
    {
        Ok(v) => {
            log::debug!("Updated {v} gfw items");
            Ok(v)
        }
        Err(e) => {
            log::error!("Error updating gfwlist: {e:?}");
            return Err(e);
        }
    }
}

pub async fn update_abp_list(proxy: &Address) -> anyhow::Result<usize> {
    match update_engine(
        abp_engine(),
        proxy,
        "https://easylist.to/easylist/easylist.txt",
        false,
    )
    .await
    {
        Ok(v) => {
            log::debug!("Updated {v} adblock items");
            Ok(v)
        }
        Err(e) => {
            log::error!("Error updating abp: {e:?}");
            return Err(e);
        }
    }
}

pub fn matches_gfw_list(addr: &Address) -> bool {
    matches_abp(gfw_engine(), addr)
}

pub fn matches_adblock_list(addr: &Address) -> bool {
    matches_abp(abp_engine(), addr)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_match_gfw_list() {
        assert!(matches_gfw_list(&"www.google.com:443".parse().unwrap()));
        assert!(matches_gfw_list(&"www.facebook.com:80".parse().unwrap()));
        assert!(matches_gfw_list(&"twitter.com:22".parse().unwrap()));
        assert!(!matches_gfw_list(&"www.qq.com:443".parse().unwrap()));
    }
}
