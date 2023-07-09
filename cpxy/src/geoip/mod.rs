use std::net::IpAddr;

use anyhow::{bail, Context};

#[derive(Clone)]
pub struct GeoIPDatabase {
    v4: Bytes,
    v6: Bytes,
}

mod country_code;

use bytes::Bytes;
pub use country_code::*;

use crate::measure_this;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
struct Record<A> {
    start_addr: A,
    end_addr: A,
    country: CountryCode,
}

impl<A> Record<A> {
    fn from_slice(buf: &[u8]) -> anyhow::Result<&[Self]>
    where
        A: Clone,
    {
        let record_size = std::mem::size_of::<Self>();
        if buf.as_ref().len() % record_size != 0 {
            bail!("Invalid buffer size, must be multiplication of {record_size}");
        }

        let raw_records = unsafe {
            std::slice::from_raw_parts(
                buf.as_ref().as_ptr() as *const Self,
                buf.as_ref().len() / record_size,
            )
        };

        Ok(raw_records)
    }

    fn find_country(sorted_list: impl AsRef<[Self]>, ip: &A) -> Option<CountryCode>
    where
        A: Ord + Copy,
    {
        match sorted_list
            .as_ref()
            .binary_search_by_key(ip, |r| r.start_addr)
        {
            Ok(idx) => Some(sorted_list.as_ref()[idx].country),
            Err(idx) if idx > 0 && ip <= &sorted_list.as_ref()[idx - 1].end_addr => {
                Some(sorted_list.as_ref()[idx - 1].country)
            }
            _ => None,
        }
    }
}

type IPv4Record = Record<[u8; 4]>;
type IPv6Record = Record<[u8; 16]>;

impl GeoIPDatabase {
    pub fn initialise_from_zstd(v4: &[u8], v6: &[u8]) -> anyhow::Result<Self> {
        measure_this!("GeoIPDatabase::initialise_from_zstd");

        let v4 = Bytes::from(zstd::decode_all(v4).context("Decoding v4 address")?);
        let v6 = Bytes::from(zstd::decode_all(v6).context("Decoding v6 address")?);

        let _ = IPv4Record::from_slice(&v4).context("Decoding v4 rcord")?;
        let _ = IPv6Record::from_slice(&v6).context("Decoding v6 rcord")?;

        Ok(Self { v4, v6 })
    }

    fn v4_records(&self) -> &[IPv4Record] {
        IPv4Record::from_slice(&self.v4).unwrap()
    }

    fn v6_records(&self) -> &[IPv6Record] {
        IPv6Record::from_slice(&self.v6).unwrap()
    }

    pub fn find_country(&self, ip: &IpAddr) -> Option<CountryCode> {
        measure_this!("GeoIPDatabase::find_country: {ip:?}");

        match ip {
            IpAddr::V4(ip) => Record::find_country(self.v4_records(), &ip.octets()),
            IpAddr::V6(ip) => Record::find_country(self.v6_records(), &ip.octets()),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_find_ip() {
        let _ = dotenvy::dotenv();
        let _ = env_logger::try_init();

        let db = GeoIPDatabase::initialise_from_zstd(
            include_bytes!("ipv4.zstd"),
            include_bytes!("ipv6.zstd"),
        )
        .expect("To initialise db");

        assert_eq!(db.find_country(&"142.250.67.4".parse().unwrap()), None);
        assert_eq!(
            db.find_country(&"219.159.81.138".parse().unwrap()),
            Some("cn".parse().unwrap())
        );
        assert_eq!(db.find_country(&"122.61.248.102".parse().unwrap()), None);
        assert_eq!(
            db.find_country(&"2409:8a5c:c434:f610:56f6:c5ff:fef3:2b9e".parse().unwrap()),
            Some("cn".parse().unwrap())
        );
    }
}
