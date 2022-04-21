use anyhow::bail;

use crate::rt::mpsc::Receiver;

pub async fn serve_udp_direct(_rx: Receiver<Vec<u8>>) -> anyhow::Result<()> {
    bail!("Unimplemented")
}
