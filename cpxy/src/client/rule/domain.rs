use regex::Regex;

use crate::geoip::CountryCode;

use super::{LineNumber, RuleMatcher};

pub enum DomainRule {
    CountryList(CountryCode),
    Regex(Regex),
}

impl RuleMatcher for DomainRule {
    fn new<'a>(buf: &'a [u8], line_number: &mut LineNumber) -> anyhow::Result<(Self, &'a [u8])>
    where
        Self: Sized,
    {
        todo!()
    }

    fn matches<'a>(&'a self, req: &crate::protocol::ProxyRequest) -> Option<LineNumber> {
        todo!()
    }
}
