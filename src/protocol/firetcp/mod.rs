mod cipher;
mod proto;
pub mod server;

pub use proto::FireTcp;

#[cfg(test)]
mod test {
    use smol::future::block_on;

    use crate::{
        protocol::test::{test_protocol_http, test_protocol_tcp},
        test::create_tcp_server,
    };

    use super::{proto::FireTcp, server::run_server};

    #[test]
    fn protocol_works() {
        block_on(async move {
            let (server, server_addr) = create_tcp_server().await;
            let _task = run_server(server);

            let protocol = FireTcp::new(server_addr.into());

            test_protocol_tcp(&protocol).await;
            test_protocol_http(&protocol).await;
        })
    }
}
