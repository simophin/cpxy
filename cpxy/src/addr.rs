use anyhow::{bail, Context};
use derive_more::{Deref, Display, From};
use either::Either;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use socks5_impl::protocol::Address as RealAddress;
use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[derive(
    Clone, Debug, PartialEq, Eq, Deref, DeserializeFromStr, SerializeDisplay, Display, From,
)]
pub struct Address(RealAddress);

impl Address {
    pub fn domain_or_ip(&self) -> Either<(&str, u16), &SocketAddr> {
        match &self.0 {
            RealAddress::SocketAddress(addr) => Either::Right(addr),
            RealAddress::DomainAddress(addr, port) => Either::Left((addr.as_ref(), *port)),
        }
    }

    pub fn domain(&self) -> Option<&str> {
        match &self.0 {
            RealAddress::SocketAddress(_) => None,
            RealAddress::DomainAddress(addr, _) => Some(addr.as_ref()),
        }
    }

    pub fn ip(&self) -> Option<IpAddr> {
        match &self.0 {
            RealAddress::SocketAddress(addr) => Some(addr.ip()),
            RealAddress::DomainAddress(_, _) => None,
        }
    }

    pub fn host(&self) -> Cow<str> {
        match &self.0 {
            RealAddress::SocketAddress(addr) => Cow::Owned(addr.ip().to_string()),
            RealAddress::DomainAddress(addr, _) => Cow::Borrowed(addr.as_str()),
        }
    }

    pub fn port(&self) -> u16 {
        match &self.0 {
            RealAddress::SocketAddress(addr) => addr.port(),
            RealAddress::DomainAddress(_, port) => *port,
        }
    }
}

impl From<SocketAddr> for Address {
    fn from(value: SocketAddr) -> Self {
        Self(value.into())
    }
}

impl From<Address> for RealAddress {
    fn from(value: Address) -> Self {
        value.0
    }
}

impl Default for Address {
    fn default() -> Self {
        RealAddress::unspecified().into()
    }
}

impl FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut splits = s.splitn(2, ':');
        match (splits.next(), splits.next()) {
            (Some(host), Some(port)) if host.len() > 0 && port.len() > 0 => {
                let port: u16 = port.parse().context("Invalid port")?;
                match IpAddr::from_str(host) {
                    Ok(addr) => Ok(Self(RealAddress::SocketAddress(SocketAddr::new(
                        addr, port,
                    )))),
                    Err(_) => Ok(Self(RealAddress::DomainAddress(host.to_string(), port))),
                }
            }

            _ => bail!("Invalid address {s}"),
        }
    }
}
