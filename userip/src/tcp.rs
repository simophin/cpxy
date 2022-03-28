use std::{fmt::Debug, net::SocketAddr};

use derive_more::Deref;
use smoltcp::{
    phy::ChecksumCapabilities,
    wire::{IpProtocol, TcpControl, TcpPacket as WireTcpPacket, TcpRepr, TcpSeqNumber},
};

use super::IpPacket;
use crate::buf::Pool;

#[derive(Debug)]
pub struct IpPacketPayloadRef(IpPacket);

impl AsRef<[u8]> for IpPacketPayloadRef {
    fn as_ref(&self) -> &[u8] {
        self.0.payload()
    }
}

#[derive(Deref)]
pub struct TcpPacket(WireTcpPacket<IpPacketPayloadRef>);

impl Debug for TcpPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl TcpPacket {
    pub fn new_unchecked(pkt: IpPacket) -> Self {
        Self(WireTcpPacket::new_unchecked(IpPacketPayloadRef(pkt)))
    }

    pub fn new_checked(pkt: IpPacket) -> Result<Self, IpPacket> {
        if WireTcpPacket::new_checked(pkt.payload()).is_err() {
            return Err(pkt);
        }

        Ok(Self::new_unchecked(pkt))
    }
}

pub fn build_tcp(
    orig: &Pool<Vec<u8>>,
    src: SocketAddr,
    dst: SocketAddr,
    build: impl FnOnce(&mut TcpRepr),
) -> IpPacket {
    let mut tcp = TcpRepr {
        src_port: src.port(),
        dst_port: dst.port(),
        control: TcpControl::None,
        seq_number: TcpSeqNumber::default(),
        ack_number: None,
        window_len: 0,
        window_scale: None,
        max_seg_size: None,
        sack_permitted: false,
        sack_ranges: [None; 3],
        payload: &[],
    };

    build(&mut tcp);

    IpPacket::build(
        orig,
        src.ip(),
        dst.ip(),
        tcp.buffer_len(),
        IpProtocol::Tcp,
        |out| {
            let mut tcp_packet = WireTcpPacket::new_unchecked(out);
            tcp.emit(
                &mut tcp_packet,
                &src.ip().into(),
                &dst.ip().into(),
                &ChecksumCapabilities::default(),
            )
        },
    )
}

pub fn ack(
    pool: &Pool<Vec<u8>>,
    src: SocketAddr,
    dst: SocketAddr,
    seq: TcpSeqNumber,
    ack: TcpSeqNumber,
) -> IpPacket {
    build_tcp(pool, src, dst, |tcp| {
        tcp.seq_number = seq;
        tcp.ack_number = Some(ack);
    })
}

pub fn fin(pool: &Pool<Vec<u8>>, src: SocketAddr, dst: SocketAddr, seq: TcpSeqNumber) -> IpPacket {
    build_tcp(pool, src, dst, |tcp| {
        tcp.seq_number = seq;
        tcp.control = TcpControl::Fin;
    })
}
