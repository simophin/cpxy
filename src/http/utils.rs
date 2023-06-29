use httparse::Header;

pub trait WithHeaders {
    fn get_headers(&self) -> &[Header];

    fn get_header(&self, name: &str) -> Option<&[u8]> {
        self.get_headers()
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value)
    }

    fn get_header_text(&self, name: &str) -> Option<&str> {
        self.get_header(name)
            .and_then(|v| std::str::from_utf8(v).ok())
    }
}

impl<'a, 'b> WithHeaders for httparse::Request<'a, 'b> {
    fn get_headers(&self) -> &[Header] {
        self.headers
    }
}

impl<'a, 'b> WithHeaders for httparse::Response<'a, 'b> {
    fn get_headers(&self) -> &[Header] {
        self.headers
    }
}
