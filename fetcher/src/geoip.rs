use std::{collections::BTreeSet, io::Write, net::IpAddr};

use super::{download_and_insert, CountryCode};
use ipnetwork::IpNetwork;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Record<A> {
    start_addr: A,
    end_addr: A,
    country: CountryCode,
}

impl<A: AsRef<[u8]>> Record<A> {
    fn write_to(&self, w: &mut impl Write) {
        w.write_all(self.start_addr.as_ref())
            .expect("to write start ip");
        w.write_all(self.end_addr.as_ref())
            .expect("to write start ip");
        w.write_all(&self.country.0).expect("to write country");
    }
}

pub fn download_and_save_geoips(
    ipv4_writer: &mut impl Write,
    ipv6_writer: &mut impl Write,
    url: &str,
    extract: impl Fn(&str) -> Option<(IpNetwork, CountryCode)>,
) {
    let mut n = 0usize;

    let mut v4: BTreeSet<Record<[u8; 4]>> = Default::default();
    let mut v6: BTreeSet<Record<[u8; 16]>> = Default::default();

    download_and_insert(url, |line| {
        let (network, country) = match extract(&line) {
            Some((network, country)) => (network, country),
            None => {
                println!("Unknown {line}");
                return;
            }
        };

        match (network.network(), network.broadcast()) {
            (IpAddr::V4(start), IpAddr::V4(end)) => {
                v4.insert(Record {
                    start_addr: start.octets(),
                    end_addr: end.octets(),
                    country,
                });
            }
            (IpAddr::V6(start), IpAddr::V6(end)) => {
                v6.insert(Record {
                    start_addr: start.octets(),
                    end_addr: end.octets(),
                    country,
                });
            }
            _ => panic!("Invalid network type: {line}"),
        }
        n += 1;
    });

    for record in v4 {
        record.write_to(ipv4_writer);
    }

    for record in v6 {
        record.write_to(ipv6_writer);
    }

    ipv4_writer.flush().expect("to flush v4");
    ipv6_writer.flush().expect("to flush v6");

    println!("Processed {n} IPs");
}
