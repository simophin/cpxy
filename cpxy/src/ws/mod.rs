use base64::Engine;
use sha1_smol::Sha1;

pub mod client;
pub mod server;

use base64::engine::general_purpose::STANDARD as B64;

fn hash_ws_key(key: impl AsRef<str>) -> String {
    let mut m = Sha1::new();
    m.update(key.as_ref().as_bytes());
    m.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    B64.encode(m.digest().bytes())
}
