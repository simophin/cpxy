use tls_parser::{
    parse_tls_extension, parse_tls_plaintext, SNIType, TlsExtension, TlsMessage,
    TlsMessageHandshake,
};

pub fn extract_http_host_header(data: &[u8]) -> Option<&str> {
    #[derive(Debug, Clone, Copy)]
    enum ParseState {
        Idle,
        Matches(usize),
        FirstNewLine,
    }

    use ParseState::*;

    let value_start_index = data
        .iter()
        .enumerate()
        .scan(Idle, |acc, (i, c)| match &acc {
            FirstNewLine | Idle if c.eq_ignore_ascii_case(&b'H') => {
                *acc = Matches(1);
                Some(-1isize)
            }
            Matches(1) if c.eq_ignore_ascii_case(&b'o') => {
                *acc = Matches(2);
                Some(-1)
            }
            Matches(2) if c.eq_ignore_ascii_case(&b's') => {
                *acc = Matches(3);
                Some(-1)
            }
            Matches(3) if c.eq_ignore_ascii_case(&b't') => {
                *acc = Matches(4);
                Some(-1)
            }
            Matches(4) if c == &b':' => Some(i as isize + 1),
            FirstNewLine if c == &b'\n' => None,
            _ if c == &b'\n' => {
                *acc = FirstNewLine;
                Some(-1)
            }
            _ => {
                *acc = Idle;
                Some(-1)
            }
        })
        .filter(|s| *s >= 0)
        .map(|s| s as usize)
        .next()?;

    let value_length = (&data[value_start_index..])
        .iter()
        .position(|c| c == &b'\n')?;

    std::str::from_utf8(&data[value_start_index..value_start_index + value_length])
        .ok()
        .map(|s| s.trim())
}

pub fn extract_ssl_sni_host(data: &[u8]) -> Option<&str> {
    let mut rem = data;
    while rem.len() > 0 {
        let (r, record) = parse_tls_plaintext(rem).ok()?;
        rem = r;
        let client_hello = record
            .msg
            .into_iter()
            .filter_map(|r| match r {
                TlsMessage::Handshake(hs) => Some(hs),
                _ => None,
            })
            .filter_map(|hs| match hs {
                TlsMessageHandshake::ClientHello(c) => Some(c),
                _ => None,
            })
            .next();

        if let Some(client_hello) = client_hello {
            let mut ext = client_hello.ext.unwrap_or_default();
            while ext.len() > 0 {
                match parse_tls_extension(ext) {
                    Ok((_, TlsExtension::SNI(sni)))
                        if !sni.is_empty() && sni[0].0 == SNIType::HostName =>
                    {
                        return std::str::from_utf8(sni[0].1).ok();
                    }
                    Ok((r, _)) => {
                        ext = r;
                    }
                    Err(_) => return None,
                };
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_http_host_works() {
        let test_hdr = r#"GET /
        Host: www.google.com
        Accept: application/json
        "#;
        assert_eq!(
            Some("www.google.com"),
            extract_http_host_header(test_hdr.as_bytes())
        );
    }

    #[test]
    fn extract_tls_sni_works() {
        assert_eq!(
            Some("www.gstatic.com"),
            extract_ssl_sni_host(include_bytes!("test/raw_tls_packet.bin"))
        )
    }
}
