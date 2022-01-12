use crate::cursor::Cursor;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn copy_with_cursor<const N: usize>(
    cursor: &mut Cursor<N>,
    rx: &mut (impl AsyncRead + Unpin + ?Sized),
    tx: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    loop {
        let bytes_read = rx.read(cursor.remaining_mut()).await?;
        if bytes_read == 0 {
            return Ok(());
        }

        cursor.move_position(bytes_read);
        tx.write_all(cursor.used()).await?;
        cursor.reset();
    }
}
