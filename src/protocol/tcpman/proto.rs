use std::{borrow::Cow, io::IoSlice, net::SocketAddr};

use anyhow::{bail, Context};
use bytes::{Buf, Bytes};
use enum_primitive_derive::Primitive;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use num_traits::FromPrimitive;
use smallvec::{smallvec, SmallVec};

use crate::{socks5::Address, utils::new_vec_uninitialised};

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Response<'a> {
    Success {
        initial_reply: Option<(SocketAddr, &'a [u8])>,
    },
    Fail {
        msg: Option<&'a str>,
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

#[derive(Debug, Primitive)]
#[repr(u8)]
enum ResponseType {
    SuccessWithInitReply = 0,
    Success = 1,
    Fail = 2,
    FailWithMsg = 3,
}

impl<'a> Response<'a> {
    pub fn parse(mut buf: &'a [u8]) -> anyhow::Result<Self> {
        if buf.len() < 1 {
            bail!("Invalid buf len");
        }

        match ResponseType::from_u8(buf.get_u8()).context("Reading response type")? {
            ResponseType::Success => Ok(Self::Success {
                initial_reply: None,
            }),
            ResponseType::SuccessWithInitReply => {
                let (offset, addr) = Address::parse(buf)
                    .context("Parsing initial address")?
                    .context("Expecting complete address")?;
                buf.advance(offset);
                let addr = match addr {
                    Address::IP(a) => a,
                    v => bail!("Unsupported address type {v}"),
                };
                Ok(Self::Success {
                    initial_reply: Some((addr, buf)),
                })
            }
            ResponseType::Fail => Ok(Self::Fail { msg: None }),
            ResponseType::FailWithMsg => {
                let msg = std::str::from_utf8(buf).context("Parsing error message")?;
                Ok(Self::Fail { msg: Some(msg) })
            }
        }
    }

    pub fn to_vec(self) -> SmallVec<[u8; 1]> {
        match self {
            Response::Success {
                initial_reply: Some((addr, init_data)),
            } => {
                let addr = Address::from(addr);
                let mut buf = SmallVec::with_capacity(1 + addr.write_len() + init_data.len());
                buf.push(ResponseType::SuccessWithInitReply as u8);
                addr.write_to(&mut buf).unwrap();
                buf.extend_from_slice(init_data);
                buf
            }
            Response::Success { .. } => {
                smallvec![ResponseType::Success as u8]
            }
            Response::Fail { msg: Some(msg) } => {
                let mut buf = SmallVec::with_capacity(1 + msg.as_bytes().len());
                buf.push(ResponseType::FailWithMsg as u8);
                buf.extend_from_slice(msg.as_bytes());
                buf
            }
            Response::Fail { .. } => smallvec![ResponseType::Fail as u8],
        }
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

    fn test_response(r: &Response<'_>) {
        let buf = r.clone().to_vec();
        assert_eq!(r, &Response::parse(&buf).expect("To parse request"));
    }

    #[test]
    fn response_parsing_works() {
        test_response(&Response::Success {
            initial_reply: Some(()),
        })
    }
}
