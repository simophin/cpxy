use crate::socks5::Address;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::net::SocketAddr;

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum Request<'a> {
    Init {
        initial_dst: Address<'a>,
        initial_data: Cow<'a, [u8]>,
        one_off: bool,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Response<'a> {
    SingleReply {
        data: Cow<'a, [u8]>,
        src: SocketAddr,
    },
    Redirect {
        port: u16,
    },
}
