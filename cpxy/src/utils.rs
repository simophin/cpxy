use anyhow::{anyhow, Context};
use bytes::{Buf, BufMut};
use pin_project_lite::pin_project;
use serde::{de::DeserializeOwned, Serialize};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use tokio::io::{copy, split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
    let (mut d1r, mut d1w) = split(d1);
    let (mut d2r, mut d2w) = split(d2);

    let task1 = async move {
        if let Some(count) = d1d2_count {
            let _ = copy_with_stats(d1r, d2w, count.as_ref()).await?;
        } else {
            let _ = copy(&mut d1r, &mut d2w).await?;
        }
        anyhow::Result::<()>::Ok(())
    };

    let task2 = async move {
        if let Some(count) = d2d1_count {
            let _ = copy_with_stats(d2r, d1w, count.as_ref()).await?;
        } else {
            let _ = copy(&mut d2r, &mut d1w).await?;
        }
        anyhow::Result::<()>::Ok(())
    };

    race(task1, task2).await
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

pin_project! {
    struct Race<F1, F2, T> {
        #[pin]
        f1: F1,

        #[pin]
        f2: F2,

        _t: PhantomData<T>,
    }
}

impl<T, F1: Future<Output = T>, F2: Future<Output = T>> Future for Race<F1, F2, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match (this.f1.poll(cx), this.f2.poll(cx)) {
            (Poll::Ready(v), _) | (_, Poll::Ready(v)) => Poll::Ready(v),
            _ => Poll::Pending,
        }
    }
}

pub fn race<T>(
    f1: impl Future<Output = T>,
    f2: impl Future<Output = T>,
) -> impl Future<Output = T> {
    Race {
        f1,
        f2,
        _t: Default::default(),
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

    #[tokio::test]
    async fn test_lengthed_encoding() {
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
    }
}
