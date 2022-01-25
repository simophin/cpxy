use bytes::Buf;
use std::borrow::Cow;
use std::mem::size_of;
use std::net::IpAddr;
use std::ptr::slice_from_raw_parts;

#[repr(C)]
struct Record<const N: usize> {
    ip_start: [u8; N],
    ip_end: [u8; N],
    code: [u8; 2],
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

pub fn find_country_by_ip(addr: IpAddr) -> Option<Cow<'static, str>> {
    match addr {
        IpAddr::V4(addr) => {
            let records = v4_records();
            let addr = u32::from_be_bytes(addr.octets());
            match records.binary_search_by_key(&addr, |r| r.get_ip_start()) {
                Ok(index) => Some(String::from_utf8_lossy(records[index].code.as_slice())),
                Err(index) => {
                    if index > 0 && addr <= records[index - 1].get_ip_end() {
                        Some(String::from_utf8_lossy(records[index - 1].code.as_slice()))
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
                Ok(index) => Some(String::from_utf8_lossy(records[index].code.as_slice())),
                Err(index) => {
                    if index > 0 && addr <= records[index - 1].get_ip_end() {
                        Some(String::from_utf8_lossy(records[index - 1].code.as_slice()))
                    } else {
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::borrow::Cow;
    use std::time::Instant;

    #[test]
    fn test_lookup() {
        let needle_nz: IpAddr = "122.61.248.102".parse().unwrap();
        let needle_us: IpAddr = "2603:c022:4000:5e00:dfd8:70ee:3b1:2e52".parse().unwrap();
        let start = Instant::now();
        assert_eq!(find_country_by_ip(needle_nz), Some(Cow::Borrowed("NZ")));
        assert_eq!(find_country_by_ip(needle_us), Some(Cow::Borrowed("US")));

        println!("Lookup takes {} microseconds", start.elapsed().as_micros())
    }
}
