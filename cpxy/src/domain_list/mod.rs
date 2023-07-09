use anyhow::Context;
use bytes::Bytes;

use crate::{geoip::CountryCode, measure_this};

struct Record<'a> {
    domain: &'a str,
    country: CountryCode,
}

impl<'a> Record<'a> {
    fn new(data: &'a [u8]) -> anyhow::Result<Self> {
        let data = std::str::from_utf8(data).context("converting to utf-8")?;
        let mut parts = data.splitn(2, |b| b == ',');
        let domain = parts
            .next()
            .with_context(|| format!("expecting domain in {data}"))?;

        let country = parts
            .next()
            .with_context(|| format!("expecting country in {data}"))?
            .parse()
            .context("parsing country code")?;

        Ok(Self { domain, country })
    }
}

pub struct DomainListRepository {
    _data: Bytes,
    sorted_records: Vec<Record<'static>>,
}

impl DomainListRepository {
    pub fn initialise_from_zstd(data: &[u8]) -> anyhow::Result<Self> {
        measure_this!("DomainListRepository::initialise_from_zstd");
        let data = Bytes::from(zstd::decode_all(data).context("Decoding domain list")?);

        let mut sorted_records = Vec::new();
        for line in data.split(|b| *b == b'\n') {
            let static_line = unsafe { std::slice::from_raw_parts(line.as_ptr(), line.len()) };
            if static_line.is_empty() {
                continue;
            }

            sorted_records.push(Record::new(static_line)?);
        }

        Ok(Self {
            _data: data,
            sorted_records,
        })
    }

    pub fn find_country(&self, base_domain: &str) -> Option<CountryCode> {
        measure_this!("DomainListRepository::find_country({base_domain})");
        let result = self
            .sorted_records
            .binary_search_by_key(&base_domain, |r| r.domain)
            .map(|s| self.sorted_records[s].country)
            .ok();
        result
    }

    // Find country code for a domain, or any of its parent domain.
    pub fn find_country_recusive(&self, domain: &str) -> Option<CountryCode> {
        measure_this!("DomainListRepository::find_country_recusive({domain})");
        let mut domain = domain;
        while !domain.is_empty() {
            if let Some(country) = self.find_country(domain) {
                return Some(country);
            }

            match domain.find('.') {
                Some(pos) if pos < domain.len() - 1 => {
                    domain = &domain[pos + 1..];
                }
                _ => return None,
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_country_works() {
        let _ = dotenvy::dotenv();
        let _ = env_logger::try_init();

        let repo = DomainListRepository::initialise_from_zstd(include_bytes!("bundled_list.zstd"))
            .expect("to initialise");

        assert_eq!(repo.find_country("baidu.com"), Some("cn".parse().unwrap()));
        assert_eq!(repo.find_country("qq.com"), Some("cn".parse().unwrap()));
        assert_eq!(repo.find_country("cctv.com"), Some("cn".parse().unwrap()));
        assert_eq!(repo.find_country("www.qq.com"), None);
        assert_eq!(repo.find_country("google.com"), None);
    }

    #[test]
    fn find_country_recursively_works() {
        let _ = dotenvy::dotenv();
        let _ = env_logger::try_init();

        let repo = DomainListRepository::initialise_from_zstd(include_bytes!("bundled_list.zstd"))
            .expect("to initialise");

        assert_eq!(
            repo.find_country_recusive("baidu.com"),
            Some("cn".parse().unwrap())
        );
        assert_eq!(
            repo.find_country_recusive("qq.com"),
            Some("cn".parse().unwrap())
        );
        assert_eq!(
            repo.find_country_recusive("cctv.com"),
            Some("cn".parse().unwrap())
        );

        assert_eq!(
            repo.find_country_recusive("www.baidu.com"),
            Some("cn".parse().unwrap())
        );
        assert_eq!(
            repo.find_country_recusive("www.qq.com"),
            Some("cn".parse().unwrap())
        );
        assert_eq!(
            repo.find_country_recusive("www.cctv.com"),
            Some("cn".parse().unwrap())
        );
        assert_eq!(repo.find_country_recusive("google.com"), None);
    }
}
