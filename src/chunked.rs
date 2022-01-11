use futures::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use httparse::Status;
use std::cmp::min;
use tide::log;

#[derive(Debug)]
enum ChunkStatus {
    Chunking { remaining_size: usize },
    EndOfChunkPendingCr,
    EndOfChunkPendingLf,
    PartialChunkLength { line_buf: Vec<u8> },
    Idle,
}

pub async fn copy_chunked_to_raw(
    r: &mut (impl AsyncBufRead + Unpin + ?Sized),
    w: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    let mut status = ChunkStatus::Idle;

    loop {
        log::debug!("ChunkStatus {:?}", status);
        let buf = r.fill_buf().await?;
        let buf_len = buf.len();
        if buf_len == 0 {
            break;
        }

        status = match status {
            ChunkStatus::Chunking { mut remaining_size } => {
                let len = min(remaining_size, buf.len());
                log::debug!("Received {} bytes from chunked", len);
                w.write_all(&buf[..len]).await?;
                r.consume_unpin(len);
                remaining_size -= len;
                if remaining_size == 0 {
                    ChunkStatus::EndOfChunkPendingCr
                } else {
                    ChunkStatus::Chunking { remaining_size }
                }
            }
            ChunkStatus::EndOfChunkPendingCr => {
                if buf.len() >= 2 && &buf[..2] == b"\r\n" {
                    r.consume_unpin(2);
                    ChunkStatus::Idle
                } else if buf[0] == b'\r' {
                    r.consume_unpin(1);
                    ChunkStatus::EndOfChunkPendingLf
                } else {
                    return Err(anyhow::anyhow!("Invalid new lines after chunk"));
                }
            }
            ChunkStatus::EndOfChunkPendingLf => {
                if buf[0] == b'\n' {
                    r.consume_unpin(1);
                    ChunkStatus::Idle
                } else {
                    return Err(anyhow::anyhow!("Invalid new lines after chunk"));
                }
            }
            ChunkStatus::Idle => {
                match httparse::parse_chunk_size(buf).map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid chunk size, buf = {:?}",
                        String::from_utf8_lossy(buf)
                    )
                })? {
                    Status::Complete((chuck_start_offset, chuck_size)) => {
                        r.consume_unpin(chuck_start_offset);
                        log::debug!("Got chunk sized {}", chuck_size);
                        ChunkStatus::Chunking {
                            remaining_size: chuck_size as usize,
                        }
                    }
                    Status::Partial => {
                        let line_buf = buf.to_vec();
                        r.consume_unpin(buf_len);
                        ChunkStatus::PartialChunkLength { line_buf }
                    }
                }
            }
            ChunkStatus::PartialChunkLength { mut line_buf } => {
                match buf.iter().position(|x| *x == b'\r') {
                    Some(i) if i < buf.len() - 1 && buf[i + 1] == b'\n' => {
                        line_buf.extend_from_slice(&buf[..(i + 2)]);
                        r.consume_unpin(i + 2);
                        match httparse::parse_chunk_size(&line_buf)
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?
                        {
                            Status::Complete((_, chuck_size)) => {
                                log::debug!("Got chunk sized {}", chuck_size);
                                ChunkStatus::Chunking {
                                    remaining_size: chuck_size as usize,
                                }
                            }
                            Status::Partial => {
                                return Err(anyhow::anyhow!(
                                    "Unexpected partial chunk length: line = {}",
                                    String::from_utf8_lossy(&line_buf)
                                ));
                            }
                        }
                    }
                    _ => {
                        line_buf.extend_from_slice(buf);
                        r.consume_unpin(buf_len);
                        ChunkStatus::PartialChunkLength { line_buf }
                    }
                }
            }
        }
    }

    Ok(())
}

pub async fn copy_raw_to_chunked(
    r: &mut (impl AsyncBufRead + Unpin + ?Sized),
    w: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut len_line = Vec::new();
    loop {
        len_line.clear();
        let buf = r.fill_buf().await?;
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(());
        }
        log::debug!("Writing {} bytes to chunked", buf_len);
        write!(&mut len_line, "{:X}\r\n", buf_len)?;
        w.write_all(&len_line).await?;
        w.write_all(buf).await?;
        w.write_all(b"\r\n").await?;
        r.consume_unpin(buf_len);
    }
}

#[cfg(test)]
mod test {
    use crate::chunked::{copy_chunked_to_raw, copy_raw_to_chunked};

    #[async_std::test]
    async fn test_copy() {
        tide::log::start();

        let buf = b"hello, world";
        let mut chunked: Vec<u8> = Vec::new();
        let mut unchunked: Vec<u8> = Vec::new();

        copy_raw_to_chunked(&mut buf.as_ref(), &mut chunked)
            .await
            .unwrap();
        copy_raw_to_chunked(&mut buf.as_ref(), &mut chunked)
            .await
            .unwrap();
        copy_chunked_to_raw(&mut chunked.as_ref(), &mut unchunked)
            .await
            .unwrap();
        assert_eq!(b"hello, worldhello, world".as_ref(), &unchunked);
    }
}
