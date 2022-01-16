use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn copy_io(
    mut r: impl AsyncRead + Unpin,
    mut w: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let mut buf = vec![0; 8192];
    loop {
        match r.read(buf.as_mut_slice()).await? {
            0 => return Ok(()),
            v => {
                w.write_all(&buf.as_slice()[..v]).await?;
            }
        }
    }
}
