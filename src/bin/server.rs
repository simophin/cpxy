use async_std::net::TcpListener;
use async_std::task::spawn;
use tide::log;

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    log::start();

    let bind_address = format!(
        "{}:{}",
        std::env::var("BIND_ADDRESS").unwrap_or("127.0.0.1".into()),
        std::env::var("BIND_PORT").unwrap_or("8383".into())
    );
    log::info!("Start listening on {}", bind_address);
    let listener = TcpListener::bind(bind_address).await?;

    let upstream_address = format!(
        "{}:{}",
        std::env::var("UPSTREAM_HOST").expect("UPSTREAM_HOST to be set"),
        std::env::var("UPSTREAM_PORT").expect("UPSTREAM_PORT to be set"),
    );

    loop {
        let (socket, addr) = listener.accept().await?;

        spawn(async move {});
    }
}
