use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;
use futures::{AsyncReadExt, AsyncWriteExt};
use smol_timeout::TimeoutExt;

use crate::{
    config::{ClientConfig, UpstreamConfig},
    proxy::{
        protocol::{ProxyRequest, ProxyResult},
        request_proxy_upstream_with_config,
        udp::{PacketReader, PacketWriter},
    },
};

const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_TIME: Duration = Duration::from_secs(15);

const TIMEOUT: Duration = Duration::from_secs(5);

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

    for (name, upstream) in &config.upstreams {
        println!("Perf testing upstream: {name}, {upstream:?}");
        run_upstream_test_tcp(&config, upstream).await?;
        run_upstream_test_udp(&config, upstream).await?;
    }

    Ok(())
}

async fn run_upstream_test_tcp(
    c: &ClientConfig,
    upstream_config: &UpstreamConfig,
) -> anyhow::Result<()> {
    println!("TCP: Connecting to {upstream_config:?}");
    match request_proxy_upstream_with_config(c.fwmark, upstream_config, &ProxyRequest::EchoTestTcp)
        .timeout(TIMEOUT)
        .await
        .context("TCP: Timeout requesting proxy")??
    {
        (ProxyResult::Granted { .. }, mut upstream, delay) => {
            println!("TCP: Connected to {upstream_config:?}, initial delay = {delay:?}");

            let mut buf = [0u8; 8192];
            let start = Instant::now();
            let mut total_uploaded = (0usize, Duration::default());
            let mut total_downloaded = (0usize, Duration::default());
            let mut last_print = start;

            while total_uploaded.0 < MAX_BYTES && Instant::now().duration_since(start) < MAX_TIME {
                let upload_start = Instant::now();
                let len = upstream
                    .write(&buf)
                    .timeout(TIMEOUT)
                    .await
                    .context("TCP: Timeout writing")??;
                total_uploaded = (
                    total_uploaded.0 + len,
                    total_uploaded.1 + Instant::now().duration_since(upload_start),
                );

                let download_start = Instant::now();
                let len = upstream
                    .read(&mut buf)
                    .timeout(TIMEOUT)
                    .await
                    .context("TCP: Timeout reading")??;
                total_downloaded = (
                    total_downloaded.0 + len,
                    total_downloaded.1 + Instant::now().duration_since(download_start),
                );

                let now = Instant::now();

                let duration_since_last_print = now.duration_since(last_print);

                if duration_since_last_print > Duration::from_secs(2) {
                    let down =
                        total_downloaded.0 / (total_downloaded.1.as_millis() as usize) * 1000;
                    let up = total_uploaded.0 / (total_uploaded.1.as_millis() as usize) * 1000;
                    println!(
                        "TCP: Average down: {}/s, up: {}/s",
                        format_bytes(down),
                        format_bytes(up)
                    );
                    last_print = now;
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
) -> anyhow::Result<()> {
    println!("UDP: Connecting to {upstream_config:?}");
    match request_proxy_upstream_with_config(c.fwmark, upstream_config, &ProxyRequest::EchoTestUdp)
        .timeout(TIMEOUT)
        .await
        .context("UDP: Timeout requesting proxy")??
    {
        (ProxyResult::Granted { .. }, mut upstream, delay) => {
            println!("UDP: Connected to {upstream_config:?}, initial delay = {delay:?}");

            // let (upstream_r, mut upstream_w) = upstream.split();
            let mut packet_stream = PacketReader::new();
            let mut packet_writer = PacketWriter::new();

            let start = Instant::now();
            let mut total_uploaded = (0usize, Duration::default());
            let mut total_downloaded = (0usize, Duration::default());
            let mut last_print = start;

            let test_address = "1.2.3.4:53".try_into().unwrap();

            while total_uploaded.0 < MAX_BYTES && Instant::now().duration_since(start) < MAX_TIME {
                let upload_start = Instant::now();
                let len = packet_writer
                    .write(&mut upstream, &test_address, &[0u8; 4986])
                    .timeout(TIMEOUT)
                    .await
                    .context("UDP: Timeout writing packet")??;
                total_uploaded = (
                    total_uploaded.0 + len,
                    total_uploaded.1 + Instant::now().duration_since(upload_start),
                );

                let download_start = Instant::now();
                let len = packet_stream
                    .read(&mut upstream)
                    .timeout(TIMEOUT)
                    .await
                    .context("UDP: Timeout receiving packet")?
                    .context("UDP: Error receiving packet")?
                    .0
                    .as_ref()
                    .len();
                total_downloaded = (
                    total_downloaded.0 + len,
                    total_downloaded.1 + Instant::now().duration_since(download_start),
                );

                let now = Instant::now();

                let duration_since_last_print = now.duration_since(last_print);

                if duration_since_last_print >= Duration::from_secs(2) {
                    let down =
                        total_downloaded.0 / (total_downloaded.1.as_millis() as usize) * 1000;
                    let up = total_uploaded.0 / (total_uploaded.1.as_millis() as usize) * 1000;
                    println!(
                        "UDP: Average down: {}/s, up: {}/s",
                        format_bytes(down),
                        format_bytes(up)
                    );
                    last_print = now;
                }
            }

            Ok(())
        }
        (r, _, _) => Err(r.into()),
    }
}
