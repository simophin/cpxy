use super::utils::WithHeaders;
use std::fmt::{Display, Formatter};

pub struct RequestDisplay<'a, 'b, 'c>(pub &'a httparse::Request<'b, 'c>);

impl<'a, 'b, 'c> Display for RequestDisplay<'a, 'b, 'c> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Request: method = {:?}, path = {:?}",
            self.0.method, self.0.path
        )?;

        HeadersDisplay(self.0).fmt(f)
    }
}

pub struct ResponseDisplay<'a, 'b, 'c>(pub &'a httparse::Response<'b, 'c>);

impl<'a, 'b, 'c> Display for ResponseDisplay<'a, 'b, 'c> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Response: code = {:?}, reason = {:?}",
            self.0.code, self.0.reason
        )?;

        HeadersDisplay(self.0).fmt(f)
    }
}

struct HeadersDisplay<'a, T>(&'a T);

impl<'a, T: WithHeaders> Display for HeadersDisplay<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for hdr in self.0.get_headers() {
            writeln!(
                f,
                "{}: {}",
                hdr.name,
                std::str::from_utf8(hdr.value).unwrap_or("[Invalid UTF-8]")
            )?;
        }

        Ok(())
    }
}
