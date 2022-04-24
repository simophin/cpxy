use anyhow::{anyhow, Context};
use bytes::{Buf, BufMut};
use futures::future::FusedFuture;
use futures::io::copy;
use futures::{select, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Future, FutureExt};
use serde::{de::DeserializeOwned, Serialize};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use crate::counter::Counter;

async fn copy_with_stats(
    mut r: impl AsyncRead + Unpin + Send + Sync,
    mut w: impl AsyncWrite + Unpin + Send + Sync,
    stat: &Counter,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 8192];
    loop {
        match r.read(buf.as_mut_slice()).await? {
            0 => return Ok(()),
            v => {
                stat.inc(v);
                w.write_all(&buf.as_slice()[..v])
                    .await
                    .context("Writing to")?;
            }
        }
    }
}

pub async fn copy_duplex(
    d1: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    d2: impl AsyncRead + AsyncWrite + Unpin + Send + Sync,
    d1d2_count: Option<Arc<Counter>>,
    d2d1_count: Option<Arc<Counter>>,
) -> anyhow::Result<()> {
    let (d1r, mut d1w) = d1.split();
    let (d2r, mut d2w) = d2.split();

    let task1 = async move {
        if let Some(count) = d1d2_count {
            let _ = copy_with_stats(d1r, d2w, count.as_ref()).await?;
        } else {
            let _ = copy(d1r, &mut d2w).await?;
        }
        anyhow::Result::<()>::Ok(())
    };

    let task2 = async move {
        if let Some(count) = d2d1_count {
            let _ = copy_with_stats(d2r, d1w, count.as_ref()).await?;
        } else {
            let _ = copy(d2r, &mut d1w).await?;
        }
        anyhow::Result::<()>::Ok(())
    };

    select! {
        r1 = task1.fuse() => r1,
        r2 = task2.fuse() => r2,
    }
    // race(task1.fuse(), task2.fuse()).await
}

pub fn write_bincode_lengthed(mut buf: &mut Vec<u8>, o: &impl Serialize) -> anyhow::Result<()> {
    let prev_len = buf.len();
    buf.put_u16(0);
    serde_json::to_writer(&mut buf, o)?;
    let written_len = buf.len() - prev_len - 2;
    if written_len > u16::MAX as usize {
        return Err(anyhow!("Object is too big: {written_len} > {}", u16::MAX));
    }

    (&mut buf.as_mut_slice()[prev_len..]).put_u16(written_len as u16);
    Ok(())
}

pub async fn write_bincode_lengthed_async(
    w: &mut (impl AsyncWrite + Unpin),
    o: &impl Serialize,
) -> anyhow::Result<()> {
    let mut data = Vec::new();
    write_bincode_lengthed(&mut data, o)?;
    w.write_all(data.as_slice()).await?;
    Ok(())
}

pub async fn read_bincode_lengthed_async<T: DeserializeOwned>(
    r: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<T> {
    let mut buf = Vec::with_capacity(512);
    buf.resize(2, 0);
    r.read_exact(buf.as_mut_slice()).await?;
    let len = buf.as_slice().get_u16() as usize;
    buf.resize(len, 0);
    r.read_exact(buf.as_mut_slice()).await?;
    match serde_json::from_slice(buf.as_slice()) {
        Ok(v) => Ok(v),
        Err(e) => {
            log::error!("Error decoding json: {e}");
            Err(e.into())
        }
    }
}

pub trait JsonSerializable {
    fn to_json(&self) -> Vec<u8>;
}

impl<T: Serialize> JsonSerializable for T {
    fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Encode json")
    }
}

pub fn new_vec_uninitialised<T>(len: usize) -> Vec<T> {
    let mut vec = Vec::with_capacity(len);
    unsafe { vec.set_len(vec.capacity()) };
    vec
}

pub fn new_vec_for_udp() -> Vec<u8> {
    new_vec_uninitialised(65536)
}

pub async fn race<T>(
    mut f1: impl Future<Output = T> + FusedFuture + Unpin,
    mut f2: impl Future<Output = T> + FusedFuture + Unpin,
) -> T {
    select! {
        r1 = f1 => r1,
        r2 = f2 => r2,
    }
}

pub trait VecExt {
    fn set_len_uninit(&mut self, len: usize);
    fn set_len_to_capacity(&mut self);
}

impl VecExt for Vec<u8> {
    fn set_len_uninit(&mut self, len: usize) {
        if len > self.capacity() {
            panic!(
                "Desired length {len} is greater than capacity {}",
                self.capacity()
            );
        }
        unsafe {
            self.set_len(len);
        }
    }

    fn set_len_to_capacity(&mut self) {
        self.set_len_uninit(self.capacity())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rt::block_on;

    #[test]
    fn test_lengthed_encoding() {
        block_on(async move {
            let data = "hello, world";
            let mut buf = Vec::<u8>::new();
            write_bincode_lengthed(&mut buf, &data).unwrap();

            let expected: String = read_bincode_lengthed_async(&mut buf.as_slice())
                .await
                .unwrap();
            assert_eq!(expected, data);

            buf.clear();
            write_bincode_lengthed_async(&mut buf, &data).await.unwrap();
            let expected: String = read_bincode_lengthed_async(&mut buf.as_slice())
                .await
                .unwrap();
            assert_eq!(expected, data);
        });
    }
}
