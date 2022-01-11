use async_std::net::TcpListener;
use async_std::task::spawn;
use cjk_proxy::socks5::serve_socks5;
use futures::AsyncReadExt;
use tide::log;

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    log::start();
    let listener = TcpListener::bind("127.0.0.1:8081").await?;
    loop {
        let (socket, addr) = listener.accept().await?;
        log::info!("Accepted connection from {}", addr);
        spawn(async move {
            let (mut rx, mut tx) = socket.split();
            if let Err(e) = serve_socks5(&mut rx, &mut tx).await {
                log::error! {"Error serving {}: {}", addr, e};
            }
            log::info!("Dropping {}", addr);
        });
    }
}
