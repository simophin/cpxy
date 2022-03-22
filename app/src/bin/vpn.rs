use proxy::vpn::serve_tun;
use smol::block_on;

fn main() {
    std::env::set_var("RUST_LOG", "debug");
    std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();
    block_on(serve_tun().unwrap()).unwrap()
}
