use anyhow::anyhow;
use std::net::{IpAddr, SocketAddr};

use bincode::{Decode, Encode};
use std::io::Write;
use url::Url;

use crate::geoip::{CountryCode, CountryCodeOwned};
use crate::socks5::Address;

type Headers = Vec<u8>;

#[derive(Encode, Decode, Debug)]
pub enum ProxyRequestType {
    SocksTCP(Address),
    SocksUDP(Option<Address>),
    Http(Address, Headers),
}

impl ProxyRequestType {
    pub fn from_http(r: &httparse::Request<'_, '_>) -> anyhow::Result<Self> {
        let path = r.path.ok_or_else(|| anyhow!("No path found"))?;
        let method = r.method.ok_or_else(|| anyhow!("No method found"))?;
        let url = match Url::parse(path) {
            Ok(v) if v.scheme().eq_ignore_ascii_case("http") && v.has_host() => v,
            Ok(v) => {
                return Err(anyhow!(
                    "Invalid scheme({:?}) or host({:?})",
                    v.scheme(),
                    v.host()
                ));
            }
            Err(_) => {
                return Err(anyhow!("Invalid path {path}"));
            }
        };

        let addr = format!(
            "{}:{}",
            url.host_str().unwrap(),
            url.port_or_known_default().unwrap_or(80)
        );

        let path = &path["http://".len()..];
        let path = match path.find("/") {
            Some(v) if v + 1 < path.len() => &path[v + 1..],
            _ => "/",
        };

        let mut headers = Vec::new();
        write!(&mut headers, "{method} {path} HTTP/1.1\r\n")?;

        let mut has_host = false;
        for hdr in r.headers {
            if hdr.name.eq_ignore_ascii_case("host") {
                has_host = true
            }
            headers.extend_from_slice(hdr.name.as_bytes());
            headers.extend_from_slice(b": ");
            headers.extend_from_slice(hdr.value);
            headers.extend_from_slice(b"\r\n");
        }

        if !has_host {
            headers.extend_from_slice(b"Host: ");
            headers.extend_from_slice(addr.as_bytes());
            headers.extend_from_slice(b"\r\n");
        }
        headers.extend_from_slice(b"\r\n");
        Ok(Self::Http(addr.parse()?, headers))
    }
}

#[derive(Encode, Decode, Debug)]
pub struct ProxyRequest {
    pub t: ProxyRequestType,
    pub reject: Vec<CountryCodeOwned>,
    pub accept: Vec<CountryCodeOwned>,
}

#[derive(Encode, Decode, Debug)]
pub enum ProxyResult {
    Granted {
        bound_address: SocketAddr,
    },
    ErrHostRejected {
        resolved: Vec<(IpAddr, Option<CountryCodeOwned>)>,
    },
    ErrHostNotFound,
    ErrTimeout,
    ErrGeneric {
        msg: String,
    },
}

impl std::fmt::Display for ProxyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ProxyResult {}
