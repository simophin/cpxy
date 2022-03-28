use futures::{select, FutureExt, Stream, StreamExt};

#[derive(Debug)]
pub enum WaitForResult<S1, S2, S3> {
    Stream1(S1),
    Stream2(S2),
    Stream3(S3),
}

pub async fn wait_for<S1, S2, S3>(
    stream1: &mut S1,
    stream2: &mut S2,
    stream3: &mut S3,
) -> WaitForResult<Option<S1::Item>, Option<S2::Item>, Option<S3::Item>>
where
    S1: Stream + Unpin,
    S2: Stream + Unpin,
    S3: Stream + Unpin,
{
    select! {
        s1 = stream1.next().fuse() => WaitForResult::Stream1(s1),
        s2 = stream2.next().fuse() => WaitForResult::Stream2(s2),
        s3 = stream3.next().fuse() => WaitForResult::Stream3(s3),
    }
}
