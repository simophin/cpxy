use futures::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use httparse::Status;
use std::cmp::min;

enum ChuckStatus {
    Chucking { remaining_size: usize },
    Idle,
}

pub async fn copy_chucked_to_raw(
    mut r: impl AsyncBufRead + Sized + Unpin,
    mut w: impl AsyncWrite + Sized + Unpin,
) -> anyhow::Result<()> {
    let mut status = ChuckStatus::Idle;

    loop {
        status = match status {
            ChuckStatus::Chucking { mut remaining_size } => {
                let buf = r.fill_buf().await?;
                if buf.len() == 0 {
                    break;
                }
                let len = min(remaining_size, buf.len());
                w.write_all(&buf[..len]).await?;
                r.consume_unpin(len);
                remaining_size -= len;
                if remaining_size == 0 {
                    ChuckStatus::Idle
                } else {
                    ChuckStatus::Chucking { remaining_size }
                }
            }
            ChuckStatus::Idle => {
                let buf = r.fill_buf().await?;
                if buf.len() == 0 {
                    break;
                }
                match httparse::parse_chunk_size(buf)
                    .map_err(|_| anyhow::anyhow!("Invalid chuck size"))?
                {
                    Status::Complete((chuck_start_offset, chuck_size)) => {
                        r.consume_unpin(chuck_start_offset);
                        ChuckStatus::Chucking {
                            remaining_size: chuck_size as usize,
                        }
                    }
                    Status::Partial => ChuckStatus::Idle,
                }
            }
        }
    }

    Ok(())
}

pub async fn copy_raw_to_chucked(
    mut r: impl AsyncBufRead + Sized + Unpin,
    mut w: impl AsyncWrite + Sized + Unpin,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut len_line = Vec::new();
    loop {
        len_line.clear();
        let buf = r.fill_buf().await?;
        write!(&mut len_line, "{}\r\n", buf.len())?;
        w.write_all(&len_line).await?;
        w.write_all(buf).await?;
    }
}
