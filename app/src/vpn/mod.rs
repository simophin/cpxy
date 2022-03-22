mod channel;
mod device;

use std::{collections::HashMap, net::Ipv4Addr, str::FromStr};

use anyhow::{bail, Context};
use bytes::Buf;
use futures_lite::{io::split, AsyncReadExt, AsyncWriteExt};
use libc::{ETH_P_IP, ETH_P_IPV6};
use nix::net::if_::InterfacesIter;
use smol::{
    channel::{bounded, Sender},
    spawn, Task,
};
use smoltcp::{
    iface::InterfaceBuilder,
    phy::{Medium, TunTapInterface},
    socket::{TcpSocket, TcpSocketBuffer, UdpSocket},
    storage::RingBuffer,
    wire::{IpAddress, IpCidr, IpProtocol, Ipv4Address, Ipv4Packet, Ipv6Packet, TcpPacket},
};
use tun::{Configuration, Device};

struct TcpClient {
    socket: TcpSocket<'static>,
}

pub fn serve_tun() -> anyhow::Result<Task<anyhow::Result<()>>> {
    let mut config = Configuration::default();
    let mtu = 1500usize;
    config
        .layer(tun::Layer::L3)
        .platform(|ctx: &mut tun::platform::linux::Configuration| {
            ctx.packet_information(true);
        })
        .address("192.168.30.1")
        .netmask("255.255.255.0")
        .mtu(mtu as i32)
        .up();
    let tun_device =
        device::Device::new(tun::platform::create(&config).context("Creating device")?)?;

    let (device, tx, rx) = channel::ChannelDevice::new(mtu, Medium::Ip);

    let iface = InterfaceBuilder::new(device, vec![])
        .ip_addrs(vec![IpCidr::new(
            IpAddress::Ipv4(Ipv4Address::new(192, 168, 30, 1)),
            24,
        )])
        .finalize();

    Ok(spawn(async move {
        loop {
            let mut buf = crate::buf::Buf::new_with_len(mtu + 4, mtu + 4);
        }
    }))
}

// fn handle_tcp4_packet(
//     ip: Ipv4Packet<&[u8]>,
//     tcp_clients: &mut HashMap<Ipv4Address, TcpSocket>,
//     outgoing_sender: &Sender<crate::buf::Buf>,
// ) -> anyhow::Result<()> {
//     let tcp = TcpPacket::new_checked(ip.payload())?;
//     match (tcp_clients.get_mut(&ip.src_addr()), tcp.syn()) {
//         (None, true) => {
//             let dst_addr = ip.dst_addr();
//             let dst_port = tcp.dst_port();
//             log::debug!("Received TCP connection destined: {dst_addr}");
//             let mut socket = TcpSocket::new(
//                 RingBuffer::new(vec![0u8; 4096]),
//                 RingBuffer::new(vec![0u8; 4096]),
//             );
//             socket
//                 .listen((dst_addr, dst_port))
//                 .context("Set listening state")?;

//             socket.process(cx, ip_repr, repr)
//         }
//         (Some(socket), _) => {
//             log::debug!("Received TCP pack");
//         }
//         _ => {
//             bail!("Packet state is wrong: {tcp}");
//         }
//     }
//     Ok(())
// }
