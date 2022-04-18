pub use smol::{block_on, spawn, Executor, Task, Timer};

pub mod net {
    pub use smol::net::{resolve, AsyncToSocketAddrs, TcpListener, TcpStream, UdpSocket};
}

pub mod mpsc {
    pub use smol::channel::*;
}

pub mod fs {
    pub use smol::fs::{create_dir_all, File};
}

pub use smol_timeout::TimeoutExt;
