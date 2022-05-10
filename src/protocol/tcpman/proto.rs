use anyhow::{bail, Context};
use bytes::Buf;
use enum_primitive_derive::Primitive;
use num_traits::FromPrimitive;

use crate::socks5::Address;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Request<'a> {
    TCP {
        dst: Address<'a>,
        initial_data: &'a [u8],
    },
    UDP {
        dst: Address<'a>,
        initial_data: &'a [u8],
    },
}

#[derive(Debug, Primitive)]
#[repr(u8)]
enum RequestType {
    TCP = 0,
    UDP = 1,
}

impl<'a> Request<'a> {
    pub fn parse(mut buf: &'a [u8]) -> anyhow::Result<Self> {
        if buf.len() < 1 {
            bail!("Invalid buf size");
        }
        let request_type = RequestType::from_u8(buf.get_u8()).context("Reading request_type")?;

        let (offset, dst) = Address::parse(&buf)
            .context("Parsing address")?
            .context("Expecting complete address")?;
        buf.advance(offset);

        Ok(match request_type {
            RequestType::TCP => Request::TCP {
                dst,
                initial_data: buf,
            },
            RequestType::UDP => Request::UDP {
                dst,
                initial_data: buf,
            },
        })
    }

    pub fn to_vec(self) -> Vec<u8> {
        let (t, dst, initial_data) = match self {
            Request::TCP { dst, initial_data } => (RequestType::TCP, dst, initial_data),
            Request::UDP { dst, initial_data } => (RequestType::UDP, dst, initial_data),
        };

        let mut buf = Vec::<u8>::with_capacity(1 + dst.write_len() + initial_data.len());
        buf.push(t as u8);
        dst.write_to(&mut buf).unwrap();
        buf.extend_from_slice(initial_data);
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_request(r: &Request<'_>) {
        let buf = r.clone().to_vec();
        assert_eq!(r, &Request::parse(&buf).expect("To parse request"));
    }

    #[test]
    fn request_parsing_works() {
        test_request(&Request::TCP {
            dst: "1.2.3.4:50".parse().unwrap(),
            initial_data: b"hello,world",
        });

        test_request(&Request::TCP {
            dst: "google.com:50".parse().unwrap(),
            initial_data: b"hello,world",
        });

        test_request(&Request::TCP {
            dst: "1.2.3.4:50".parse().unwrap(),
            initial_data: b"",
        });

        test_request(&Request::TCP {
            dst: "google.com:50".parse().unwrap(),
            initial_data: b"",
        });

        test_request(&Request::UDP {
            dst: "1.2.3.4:50".parse().unwrap(),
            initial_data: b"hello,world",
        });

        test_request(&Request::UDP {
            dst: "google.com:50".parse().unwrap(),
            initial_data: b"hello,world",
        });

        test_request(&Request::UDP {
            dst: "1.2.3.4:50".parse().unwrap(),
            initial_data: b"",
        });

        test_request(&Request::UDP {
            dst: "google.com:50".parse().unwrap(),
            initial_data: b"",
        });
    }
}
