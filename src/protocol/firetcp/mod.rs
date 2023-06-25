mod cipher;
mod proto;
mod pw;
pub mod server;

pub use proto::FireTcp;

#[cfg(test)]
mod test {
    use crate::{
        protocol::test::{test_protocol_http, test_protocol_tcp},
        test::create_tcp_server,
    };
    use tokio::spawn;

    use super::{proto::FireTcp, pw::PasswordedKey, server::run_server};

    #[tokio::test]
    async fn protocol_works() {
        let (server, server_addr) = create_tcp_server().await;
        let pw = PasswordedKey::new("123456");
        let _task = spawn(run_server(server, pw.clone()));

        let protocol = FireTcp::new(server_addr.into(), pw);

        test_protocol_tcp(&protocol).await;
        test_protocol_http(&protocol).await;
    }
}
