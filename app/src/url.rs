use std::borrow::Cow;

use anyhow::bail;

use crate::socks5::Address;

pub struct Url<'a> {
    pub scheme: Cow<'a, str>,
    pub host: Cow<'a, str>,
    pub port: Option<u16>,
    pub path: Cow<'a, str>,
}

pub struct HttpUrl<'a> {
    pub is_https: bool,
    pub address: Address<'a>,
    pub path: Cow<'a, str>,
}

impl<'a> TryFrom<&'a str> for HttpUrl<'a> {
    type Error = anyhow::Error;

    fn try_from(url: &'a str) -> Result<Self, Self::Error> {
        let Url {
            scheme,
            host,
            port,
            mut path,
        } = url.try_into()?;

        let (is_https, default_port) = match scheme {
            s if s.eq_ignore_ascii_case("http") => (false, 80),
            s if s.eq_ignore_ascii_case("https") => (true, 443),
            s => bail!("Unknown http scheme {s}"),
        };

        if path.is_empty() {
            path = Cow::Borrowed("/");
        }

        Ok(HttpUrl {
            is_https,
            address: (host, port.unwrap_or(default_port)).try_into()?,
            path,
        })
    }
}

impl<'a> TryFrom<&'a str> for Url<'a> {
    type Error = anyhow::Error;

    fn try_from(url: &'a str) -> Result<Self, Self::Error> {
        let (scheme, url) = match url.find("://") {
            Some(v) => (&url[..v], &url[(v + 3)..]),
            None => ("", url),
        };

        let (host_and_port, path) = match url.find('/') {
            Some(v) => (&url[..v], &url[v..]),
            None => (url, ""),
        };

        let (host, port) = match host_and_port.rfind(':') {
            Some(v) if v > 0 && v < host_and_port.len() - 1 => (
                &host_and_port[..v],
                Some((&host_and_port[v + 1..]).parse()?),
            ),
            Some(_) => bail!("Invalid host or port: {host_and_port}"),
            None => (host_and_port, None),
        };

        Ok(Self {
            scheme: Cow::Borrowed(scheme),
            host: Cow::Borrowed(host),
            port,
            path: Cow::Borrowed(path),
        })
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_decoding() {
        struct TestCase {
            input: &'static str,
            expect_error: bool,
            expect_scheme: &'static str,
            expect_host: &'static str,
            expect_port: Option<u16>,
            expect_path: &'static str,
        }

        let cases = [
            TestCase {
                input: "https://www.google.com/go",
                expect_scheme: "https",
                expect_host: "www.google.com",
                expect_port: None,
                expect_path: "/go",
                expect_error: false,
            },
            TestCase {
                input: "https://www.google.com:443/go",
                expect_scheme: "https",
                expect_host: "www.google.com",
                expect_port: Some(443),
                expect_path: "/go",
                expect_error: false,
            },
            TestCase {
                input: "https://www.google.com",
                expect_scheme: "https",
                expect_host: "www.google.com",
                expect_port: None,
                expect_path: "",
                expect_error: false,
            },
            TestCase {
                input: "https://www.google.com:80",
                expect_scheme: "https",
                expect_host: "www.google.com",
                expect_port: Some(80),
                expect_path: "",
                expect_error: false,
            },
            TestCase {
                input: "www.google.com/go",
                expect_scheme: "",
                expect_host: "www.google.com",
                expect_port: None,
                expect_path: "/go",
                expect_error: false,
            },
            TestCase {
                input: "www.google.com:80/go",
                expect_scheme: "",
                expect_host: "www.google.com",
                expect_port: Some(80),
                expect_path: "/go",
                expect_error: false,
            },
            TestCase {
                input: "www.google.com:80",
                expect_scheme: "",
                expect_host: "www.google.com",
                expect_port: Some(80),
                expect_path: "",
                expect_error: false,
            },
            TestCase {
                input: "/go",
                expect_scheme: "",
                expect_host: "",
                expect_port: None,
                expect_path: "/go",
                expect_error: false,
            },
            TestCase {
                input: "www.google.com:/go",
                expect_scheme: "",
                expect_host: "",
                expect_port: None,
                expect_path: "/go",
                expect_error: true,
            },
        ];

        for TestCase {
            input,
            expect_error,
            expect_host,
            expect_path,
            expect_port,
            expect_scheme,
        } in cases
        {
            match super::Url::try_from(input) {
                Ok(url) => {
                    assert!(!expect_error);
                    assert_eq!(expect_host, url.host);
                    assert_eq!(expect_path, url.path);
                    assert_eq!(expect_port, url.port);
                    assert_eq!(expect_scheme, url.scheme);
                }
                Err(e) => {
                    if !expect_error {
                        panic!("Error parsing {input}: {e:?}");
                    }
                }
            }
        }
    }
}
