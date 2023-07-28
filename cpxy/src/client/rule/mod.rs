use std::num::NonZeroUsize;

use crate::protocol::ProxyRequest;

mod domain;
mod ip;
mod line;
mod op;
mod parser;

pub enum Rule {
    MatchDomain(domain::DomainRule),
    MatchIP(ip::IPRule),
}

type LineNumber = NonZeroUsize;

trait RuleMatcher {
    fn new<'a>(buf: &'a [u8], line_number: &mut LineNumber) -> anyhow::Result<(Self, &'a [u8])>
    where
        Self: Sized;

    fn matches<'a>(&'a self, req: &ProxyRequest) -> Option<LineNumber>;
}
