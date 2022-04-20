use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;
use futures_lite::{io::split, AsyncReadExt, AsyncWriteExt, StreamExt};

use crate::{
    config::{ClientConfig, UpstreamConfig},
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream_with_config,
        udp::{Packet, PacketWriter},
    },
};

const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_TIME: Duration = Duration::from_secs(15);

fn format_bytes(n: usize) -> String {
    if n < 1024 * 1024 {
        return format!("{}KB", n / 1024);
    } else {
        return format!("{}MB", n / 1024 / 1024);
    }
}

pub async fn run_perf_tests(config_file: &Path) -> anyhow::Result<()> {
    let config: ClientConfig = serde_yaml::from_reader(
        std::fs::File::open(config_file)
            .with_context(|| format!("Opening config file {config_file:?}"))?,
    )?;

    let mut stdout = std::io::stdout();

    for (name, upstream) in &config.upstreams {
        writeln!(stdout, "Perf testing upstream: {name}, {upstream:?}")?;
        run_upstream_test_tcp(&config, upstream, &mut stdout).await?;
        run_upstream_test_udp(&config, upstream, &mut stdout).await?;
    }

    Ok(())
}

async fn run_upstream_test_tcp(
    c: &ClientConfig,
    upstream_config: &UpstreamConfig,
    out: &mut (impl Write + Send + Sync),
) -> anyhow::Result<()> {
    writeln!(out, "TCP: Connecting to {upstream_config:?}")?;
    match request_proxy_upstream_with_config(c.fwmark, upstream_config, &ProxyRequest::EchoTestTcp)
        .await?
    {
        (ProxyResult::Granted { .. }, mut upstream, delay) => {
            writeln!(
                out,
                "TCP: Connected to {upstream_config:?}, initial delay = {delay:?}"
            )?;

            let mut buf = [0u8; 8192];
            let start = Instant::now();
            let mut total_uploaded = (0usize, Duration::default());
            let mut total_downloaded = (0usize, Duration::default());
            let mut last_print: Option<Instant> = None;

            while total_uploaded.0 < MAX_BYTES || Instant::now().duration_since(start) < MAX_TIME {
                let upload_start = Instant::now();
                let len = upstream.write(&buf).await?;
                total_uploaded = (
                    total_uploaded.0 + len,
                    total_uploaded.1 + Instant::now().duration_since(upload_start),
                );

                let download_start = Instant::now();
                let len = upstream.read(&mut buf).await?;
                total_downloaded = (
                    total_downloaded.0 + len,
                    total_downloaded.1 + Instant::now().duration_since(download_start),
                );

                let now = Instant::now();

                let duration_since_last_print =
                    last_print.as_ref().map(|last| now.duration_since(*last));

                match duration_since_last_print {
                    Some(d) if d >= Duration::from_secs(2) => {
                        let down = total_downloaded.0 / (total_downloaded.1.as_secs() as usize);
                        let up = total_uploaded.0 / (total_uploaded.1.as_secs() as usize);
                        writeln!(
                            out,
                            "TCP: Average down: {}/s, up: {}/s",
                            format_bytes(down),
                            format_bytes(up)
                        )?;
                        last_print = Some(now);
                    }
                    _ => {}
                }
            }

            Ok(())
        }
        (r, _, _) => Err(r.into()),
    }
}

async fn run_upstream_test_udp(
    c: &ClientConfig,
    upstream_config: &UpstreamConfig,
    out: &mut (impl Write + Send + Sync),
) -> anyhow::Result<()> {
    writeln!(out, "UDP: Connecting to {upstream_config:?}")?;
    match request_proxy_upstream_with_config(c.fwmark, upstream_config, &ProxyRequest::EchoTestUdp)
        .await?
    {
        (ProxyResult::Granted { .. }, upstream, delay) => {
            writeln!(
                out,
                "UDP: Connected to {upstream_config:?}, initial delay = {delay:?}"
            )?;

            let (upstream_r, mut upstream_w) = split(upstream);
            let mut packet_stream = Packet::new_packet_stream(upstream_r, None);
            let mut packet_writer = PacketWriter::new();

            let start = Instant::now();
            let mut total_uploaded = (0usize, Duration::default());
            let mut total_downloaded = (0usize, Duration::default());
            let mut last_print: Option<Instant> = None;

            let test_address = "1.2.3.4".try_into().unwrap();

            while total_uploaded.0 < MAX_BYTES || Instant::now().duration_since(start) < MAX_TIME {
                let upload_start = Instant::now();
                let len = packet_writer
                    .write(&mut upstream_w, &test_address, &[0u8; 4986])
                    .await?;
                total_uploaded = (
                    total_uploaded.0 + len,
                    total_uploaded.1 + Instant::now().duration_since(upload_start),
                );

                let download_start = Instant::now();
                let len = packet_stream
                    .next()
                    .await
                    .context("Error receiving packet")?
                    .0
                    .as_ref()
                    .len();
                total_downloaded = (
                    total_downloaded.0 + len,
                    total_downloaded.1 + Instant::now().duration_since(download_start),
                );

                let now = Instant::now();

                let duration_since_last_print =
                    last_print.as_ref().map(|last| now.duration_since(*last));

                match duration_since_last_print {
                    Some(d) if d >= Duration::from_secs(2) => {
                        let down = total_downloaded.0 / (total_downloaded.1.as_secs() as usize);
                        let up = total_uploaded.0 / (total_uploaded.1.as_secs() as usize);
                        writeln!(
                            out,
                            "UDP: Average down: {}/s, up: {}/s",
                            format_bytes(down),
                            format_bytes(up)
                        )?;
                        last_print = Some(now);
                    }
                    _ => {}
                }
            }

            Ok(())
        }
        (r, _, _) => Err(r.into()),
    }
}
