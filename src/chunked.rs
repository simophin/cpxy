use futures::{AsyncBufRead, AsyncWrite};

pub async fn copy_chunked_to_raw(
    r: &mut (impl AsyncBufRead + Unpin + ?Sized),
    w: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    async_std::io::copy(r, w).await?;
    Ok(())
}

pub async fn copy_raw_to_chunked(
    r: &mut (impl AsyncBufRead + Unpin + ?Sized),
    w: &mut (impl AsyncWrite + Unpin + ?Sized),
) -> anyhow::Result<()> {
    copy_chunked_to_raw(r, w).await
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
