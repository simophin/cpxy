use std::net::IpAddr;

use buf::{Pool, Pooled};
use smoltcp::{
    phy::ChecksumCapabilities,
    wire::{IpProtocol, Ipv4Packet, Ipv4Repr, Ipv6Packet},
};

mod buf;
mod tcp;
mod tcp_sockets;
mod utils;

#[derive(Debug)]
pub enum IpPacket {
    V4(Ipv4Packet<Pooled<Vec<u8>>>),
    V6(Ipv6Packet<Pooled<Vec<u8>>>),
}

impl AsRef<[u8]> for IpPacket {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::V4(p) => p.as_ref(),
            Self::V6(p) => p.as_ref(),
        }
    }
}

impl IpPacket {
    pub fn src_addr(&self) -> IpAddr {
        match self {
            Self::V4(p) => IpAddr::V4(p.src_addr().into()),
            Self::V6(p) => IpAddr::V6(p.src_addr().into()),
        }
    }

    pub fn dst_addr(&self) -> IpAddr {
        match self {
            Self::V4(p) => IpAddr::V4(p.dst_addr().into()),
            Self::V6(p) => IpAddr::V6(p.dst_addr().into()),
        }
    }

    pub fn payload(&self) -> &[u8] {
        match self {
            Self::V4(p) => Ipv4Packet::new_unchecked(p.as_ref()).payload(),
            Self::V6(p) => Ipv6Packet::new_unchecked(p.as_ref()).payload(),
        }
    }

    pub fn protocol(&self) -> IpProtocol {
        match self {
            Self::V4(p) => p.protocol(),
            Self::V6(p) => p.next_header(),
        }
    }

    pub fn build(
        buf: &Pool<Vec<u8>>,
        src_addr: IpAddr,
        dst_addr: IpAddr,
        payload_len: usize,
        protocol: IpProtocol,
        f: impl FnOnce(&mut [u8]),
    ) -> Self {
        match (src_addr, dst_addr) {
            (IpAddr::V4(src), IpAddr::V4(dst)) => {
                let ip = Ipv4Repr {
                    src_addr: src.into(),
                    dst_addr: dst.into(),
                    protocol,
                    payload_len,
                    hop_limit: 64,
                };
                let mut buf = buf.allocate(ip.buffer_len() + payload_len);
                let cap = buf.capacity();
                buf.resize(cap, 0);

                let mut pkt = Ipv4Packet::new_unchecked(buf.as_mut_slice());
                ip.emit(&mut pkt, &ChecksumCapabilities::default());
                f(pkt.payload_mut());
                pkt.fill_checksum();
                Self::V4(Ipv4Packet::new_unchecked(buf))
            }
            (IpAddr::V6(src), IpAddr::V6(dst)) => {
                todo!()
            }
            _ => panic!("Invalid combination of src/dst"),
        }
    }
}
