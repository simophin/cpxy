use crate::socks5::Address;
use serde::de::{Deserialize, Error, Visitor};
use serde::{Deserializer, Serialize, Serializer};
use smol::net::resolve;
use std::fmt::{Debug, Display, Formatter};
use std::mem::size_of;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU8;
use std::ptr::slice_from_raw_parts;
use std::str::FromStr;

#[repr(C)]
struct Record<const N: usize> {
    ip_start: [u8; N],
    ip_end: [u8; N],
    code: [NonZeroU8; 2],
}

impl Record<4> {
    #[inline]
    fn get_ip_start(&self) -> u32 {
        return u32::from_be_bytes(self.ip_start);
    }

    #[inline]
    fn get_ip_end(&self) -> u32 {
        return u32::from_be_bytes(self.ip_end);
    }
}

impl Record<16> {
    #[inline]
    fn get_ip_start(&self) -> u128 {
        return u128::from_be_bytes(self.ip_start);
    }

    #[inline]
    fn get_ip_end(&self) -> u128 {
        return u128::from_be_bytes(self.ip_end);
    }
}

type V4Record = Record<4>;
type V6Record = Record<16>;

const GEO_IPV4_DATA: &'static [u8] = include_bytes!(concat!(env!("OUT_DIR"), "/geoip4.dat"));
const V4_RECORD_LEN: usize = GEO_IPV4_DATA.len() / size_of::<V4Record>();

const GEO_IPV6_DATA: &'static [u8] = include_bytes!(concat!(env!("OUT_DIR"), "/geoip6.dat"));
const V6_RECORD_LEN: usize = GEO_IPV6_DATA.len() / size_of::<V6Record>();

fn v4_records() -> &'static [V4Record] {
    unsafe { &*slice_from_raw_parts(GEO_IPV4_DATA.as_ptr() as *const V4Record, V4_RECORD_LEN) }
}

fn v6_records() -> &'static [V6Record] {
    unsafe { &*slice_from_raw_parts(GEO_IPV6_DATA.as_ptr() as *const V6Record, V6_RECORD_LEN) }
}

#[derive(Eq, Copy, Clone, Hash)]
pub struct CountryCode([NonZeroU8; 2]);

impl PartialEq for CountryCode {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice().eq_ignore_ascii_case(other.as_slice())
    }
}

impl Serialize for CountryCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(String::from_utf8_lossy(self.as_slice()).as_ref())
    }
}

struct CountryCodeVisitor;

impl<'de> Visitor<'de> for CountryCodeVisitor {
    type Value = CountryCode;

    fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        write!(formatter, "a string with two characters")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        match v.parse::<Self::Value>() {
            Ok(v) => Ok(v),
            Err(_) => return Err(serde::de::Error::custom("Invalid country code")),
        }
    }
}

impl<'de> Deserialize<'de> for CountryCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(CountryCodeVisitor)
    }
}

#[derive(Debug)]
pub enum CountryCodeParseError {
    InvalidLength,
}

impl Display for CountryCodeParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self, f)
    }
}

impl std::error::Error for CountryCodeParseError {}

impl FromStr for CountryCode {
    type Err = CountryCodeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.as_bytes().len() != 2 {
            return Err(CountryCodeParseError::InvalidLength);
        }

        let mut transformed = s
            .as_bytes()
            .iter()
            .map(|x| unsafe { NonZeroU8::new_unchecked(x.to_ascii_uppercase()) });

        Ok(Self([
            transformed.next().unwrap(),
            transformed.next().unwrap(),
        ]))
    }
}

impl CountryCode {
    fn as_slice(&self) -> &[u8] {
        unsafe { &*(&self.0 as *const [NonZeroU8; 2] as *const [u8; 2]) }
    }
}

impl Debug for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for CountryCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(String::from_utf8_lossy(self.as_slice()).as_ref())
    }
}

pub fn find_country_by_ip(addr: &IpAddr) -> Option<CountryCode> {
    match addr {
        IpAddr::V4(addr) => {
            let records = v4_records();
            let addr = u32::from_be_bytes(addr.octets());
            match records.binary_search_by_key(&addr, |r| r.get_ip_start()) {
                Ok(index) => Some(CountryCode(records[index].code)),
                Err(index) => {
                    if index > 0 && addr <= records[index - 1].get_ip_end() {
                        Some(CountryCode(records[index - 1].code))
                    } else {
                        None
                    }
                }
            }
        }

        IpAddr::V6(addr) => {
            let records = v6_records();
            let addr = u128::from_be_bytes(addr.octets());
            match records.binary_search_by_key(&addr, |r| r.get_ip_start()) {
                Ok(index) => Some(CountryCode(records[index].code)),
                Err(index) => {
                    if index > 0 && addr <= records[index - 1].get_ip_end() {
                        Some(CountryCode(records[index - 1].code))
                    } else {
                        None
                    }
                }
            }
        }
    }
}

pub async fn resolve_with_countries(addr: &Address) -> Vec<(SocketAddr, Option<CountryCode>)> {
    match addr {
        Address::IP(addr) => vec![(addr.clone(), find_country_by_ip(&addr.ip()))],
        Address::Name { host, port } => {
            let result = resolve((host.as_str(), *port))
                .await
                .ok()
                .unwrap_or_default()
                .into_iter()
                .map(|addr| (addr, find_country_by_ip(&addr.ip())))
                .collect();

            log::debug!("Resolved {host}: {result:?}");

            result
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_lookup() {
        let needle_nz: IpAddr = "122.61.248.102".parse().unwrap();
        let needle_us: IpAddr = "2603:c022:4000:5e00:dfd8:70ee:3b1:2e52".parse().unwrap();
        let start = Instant::now();
        assert_eq!(find_country_by_ip(&needle_nz), Some("NZ".parse().unwrap()));
        assert_eq!(find_country_by_ip(&needle_us), Some("US".parse().unwrap()));

        println!("Lookup takes {} microseconds", start.elapsed().as_micros());

        smol::block_on(async move {
            let addresses = resolve_with_countries(&Address::Name {
                host: "www.google.com".to_string(),
                port: 443,
            })
            .await;
            println!("Resolved addresses: {addresses:?}");
            assert!(addresses
                .iter()
                .find(|x| x.1 == Some("US".parse().unwrap()))
                .is_some());
        });
    }

    #[test]
    fn test_json() {
        let code: CountryCode = "NZ".parse().unwrap();
        let v = serde_json::to_string(&code).unwrap();
        assert_eq!(v, "\"NZ\"");
        let expect: CountryCode = serde_json::from_str(v.as_str()).unwrap();
        assert_eq!(expect, code);
    }

    #[test]
    fn test_country_code() {
        let codes: Vec<CountryCode> = vec!["nz", "US"]
            .into_iter()
            .map(|c| c.parse().unwrap())
            .collect();
        assert!(codes.contains(&"NZ".parse().unwrap()));
        assert!(codes.contains(&"US".parse().unwrap()));
        assert!(codes.contains(&"us".parse().unwrap()));
    }
}
