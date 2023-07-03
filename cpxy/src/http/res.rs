use crate::http::display::ResponseDisplay;
use anyhow::{bail, Context};
use bytes::BytesMut;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

pub async fn parse_response<T, F>(r: &mut (impl AsyncBufRead + Unpin), f: F) -> anyhow::Result<T>
where
    F: FnOnce(&httparse::Response<'_, '_>) -> anyhow::Result<T>,
{
    // Try if we can parse the http header in one go:
    let buf = r.fill_buf().await.context("Reading request")?;
    if buf.len() == 0 {
        bail!("EOF while reading request");
    }

    return match parse_response_full(buf, f)? {
        Ok((t, length)) => {
            r.consume(length);
            Ok(t)
        }

        Err(f) => parse_response_partial(r, f).await,
    };
}

fn parse_response_full<T, F>(buf: &[u8], f: F) -> anyhow::Result<Result<(T, usize), F>>
where
    F: FnOnce(&httparse::Response<'_, '_>) -> anyhow::Result<T>,
{
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut res = httparse::Response::new(&mut headers);
    return match res.parse(buf).context("Parsing http request")? {
        httparse::Status::Complete(length) => {
            log::debug!("Parsed http response: {}", ResponseDisplay(&res));
            let result = f(&res);
            result.map(|t| Ok((t, length)))
        }

        httparse::Status::Partial => Ok(Err(f)),
    };
}

async fn parse_response_partial<T, F>(
    r: &mut (impl AsyncBufRead + Unpin),
    mut f: F,
) -> anyhow::Result<T>
where
    F: FnOnce(&httparse::Response<'_, '_>) -> anyhow::Result<T>,
{
    let fill_buf = r.fill_buf().await?;
    let mut buf = BytesMut::from(fill_buf);
    r.consume(buf.len());

    while buf.len() < 65536 {
        let fill_buf = r.fill_buf().await?;
        let fill_buf_len = fill_buf.len();
        buf.extend_from_slice(fill_buf);

        match parse_response_full(&buf, f)? {
            Ok((t, length)) => {
                r.consume(buf.len() - length);
                return Ok(t);
            }
            Err(cb) => {
                r.consume(fill_buf_len);
                f = cb;
            }
        }
    }

    bail!("Excess header length")
}
