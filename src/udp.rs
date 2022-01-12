use crate::socks5::Address;
use anyhow::anyhow;
use bytes::Buf;

#[derive(Debug, Eq, PartialEq)]
pub enum Message<'a> {
    Send(&'a [u8]),
    SendTo(Address, &'a [u8]),
}

impl<'a> Message<'a> {
    pub fn parse(buf: &mut impl Buf) -> anyhow::Result<Option<Self>> {
        if !buf.has_remaining() {
            return Ok(None);
        }

        Ok(None)
    }
}
