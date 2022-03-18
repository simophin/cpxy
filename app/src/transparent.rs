#[cfg(unix)]
mod unix {
    use std::{
        net::{SocketAddr, SocketAddrV4},
        os::unix::prelude::AsRawFd,
    };

    use nix::sys::socket::{getsockopt, sockopt::OriginalDst};

    use crate::io::TcpStream;

    pub fn get_original_socket_addr(socket: &TcpStream) -> Option<SocketAddr> {
        let addr = getsockopt(socket.as_raw_fd(), OriginalDst).ok()?;

        let addr = SocketAddr::V4(SocketAddrV4::new(
            u32::from_be(addr.sin_addr.s_addr).into(),
            u16::from_be(addr.sin_port),
        ));

        match socket.local_addr() {
            Ok(v) if v == addr => return None,
            _ => {}
        };

        Some(addr)
    }
}

#[cfg(unix)]
pub use unix::get_original_socket_addr;

#[cfg(not(unix))]
pub fn get_original_socket_addr(_: &crate::io::TcpStream) -> Option<SocketAddr> {
    None
}
