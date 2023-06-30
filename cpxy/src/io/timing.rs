use std::future::Future;
use std::time::{Duration, Instant};

pub async fn time_future<F, T, E>(fut: F) -> Result<(T, Duration), E> where
    F: Future<Output=Result<T, E>> {
    let start = Instant::now();
    let data = fut.await?;
    let duration = start.elapsed();
    Ok((data, duration))
}