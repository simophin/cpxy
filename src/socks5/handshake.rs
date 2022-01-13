use super::ClientConnRequest;
use futures::{AsyncRead, AsyncWrite};

pub async fn negotiate_request(
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<ClientConnRequest<'static>> {
    let buf = [0u8; 512];

    // Wait for greeting...
    {
        let buf = buf.as_slice();
    }
    panic!()
}
