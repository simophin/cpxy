use async_std::io::BufReader;
use futures::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt};

pub async fn copy_io(
    r: impl AsyncRead + Unpin,
    mut w: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let mut r = BufReader::new(r);
    loop {
        match r.fill_buf().await? {
            buf if buf.len() > 0 => {
                let len = buf.len();
                w.write_all(buf).await?;
                r.consume_unpin(len);
            }
            _ => return Ok(()),
        }
    }
}
