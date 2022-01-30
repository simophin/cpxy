mod country_code;

pub use country_code::CountryCode;
use std::mem::size_of;
use std::net::IpAddr;

use bytes::Buf;
use lazy_static::lazy_static;
use std::slice::from_raw_parts;

#[repr(C)]
struct Record<const N: usize> {
    start: [u8; N],
    end: [u8; N],
    c: CountryCode,
}

fn load_ip_dat<const N: usize>(raw: &[u8]) -> &'static [Record<N>] {
    let len = raw.len() / size_of::<Record<N>>();
    unsafe { from_raw_parts(raw.as_ptr() as *const Record<N>, len) }
}

pub fn find_geoip(ip: &IpAddr) -> Option<CountryCode> {
    match ip {
        IpAddr::V4(addr) => {
            let needle = addr.octets().as_slice().get_u32();
            lazy_static! {
                static ref RECORDS_V4: &'static [Record<4>] =
                    load_ip_dat(include_bytes!("ipv4.dat"));
            }
            match RECORDS_V4.binary_search_by_key(&needle, |r| u32::from_be_bytes(r.start)) {
                Ok(index) => Some(RECORDS_V4[index].c),
                Err(index)
                    if index > 0
                        && needle >= u32::from_be_bytes(RECORDS_V4[index - 1].start)
                        && needle <= u32::from_be_bytes(RECORDS_V4[index - 1].end) =>
                {
                    Some(RECORDS_V4[index - 1].c)
                }
                _ => None,
            }
        }
        IpAddr::V6(addr) => {
            let needle = addr.octets().as_slice().get_u128();
            lazy_static! {
                static ref RECORDS_V6: &'static [Record<16>] =
                    load_ip_dat(include_bytes!("ipv6.dat"));
            }
            match RECORDS_V6.binary_search_by_key(&needle, |r| u128::from_be_bytes(r.start)) {
                Ok(index) => Some(RECORDS_V6[index].c),
                Err(index)
                    if index > 0
                        && needle >= u128::from_be_bytes(RECORDS_V6[index - 1].start)
                        && needle <= u128::from_be_bytes(RECORDS_V6[index - 1].end) =>
                {
                    Some(RECORDS_V6[index - 1].c)
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_ip() {
        let us: CountryCode = "US".parse().unwrap();

        assert_eq!(find_geoip(&"142.250.67.4".parse().unwrap()), Some(us));
        assert_eq!(
            find_geoip(&"219.159.81.138".parse().unwrap()),
            Some("cn".parse().unwrap())
        );
        assert_eq!(
            find_geoip(&"122.61.248.102".parse().unwrap()),
            Some("NZ".parse().unwrap())
        );
        assert_eq!(
            find_geoip(&"2001:4860:4860::8888".parse().unwrap()),
            Some(us)
        );
    }
}
