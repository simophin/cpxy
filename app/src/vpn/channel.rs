use smol::channel::{bounded, Receiver, Sender};
use smoltcp::{
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    time::Instant,
};

use crate::buf::Buf;

pub struct ChannelDevice {
    outgoing_tx: Sender<Buf>,
    incoming_rx: Receiver<Buf>,
    mtu: usize,
    medium: Medium,
}

impl ChannelDevice {
    pub fn new(mtu: usize, medium: Medium) -> (ChannelDevice, Sender<Buf>, Receiver<Buf>) {
        let (incoming_tx, incoming_rx) = bounded(64);
        let (outgoing_tx, outgoing_rx) = bounded(64);
        (
            ChannelDevice {
                outgoing_tx,
                incoming_rx,
                mtu,
                medium,
            },
            incoming_tx,
            outgoing_rx,
        )
    }
}

pub struct AsyncRxToken(Option<Buf>);

impl RxToken for AsyncRxToken {
    fn consume<R, F>(mut self, _timestamp: Instant, f: F) -> smoltcp::Result<R>
    where
        F: FnOnce(&mut [u8]) -> smoltcp::Result<R>,
    {
        if let Some(mut buf) = self.0 {
            f(&mut buf)
        } else {
            Err(smoltcp::Error::Exhausted)
        }
    }
}

pub struct AsyncTxToken(Sender<Buf>);

impl TxToken for AsyncTxToken {
    fn consume<R, F>(self, timestamp: Instant, len: usize, f: F) -> smoltcp::Result<R>
    where
        F: FnOnce(&mut [u8]) -> smoltcp::Result<R>,
    {
        let mut buf = Buf::new_with_len(len, len);
        let r = f(&mut buf)?;
        self.0
            .try_send(buf)
            .map_err(|e| smoltcp::Error::Exhausted)?;
        Ok(r)
    }
}

impl<'a> Device<'a> for ChannelDevice {
    type RxToken = AsyncRxToken;
    type TxToken = AsyncTxToken;

    fn receive(&'a mut self) -> Option<(Self::RxToken, Self::TxToken)> {
        let buf = self.incoming_rx.try_recv().ok();
        Some((AsyncRxToken(buf), AsyncTxToken(self.outgoing_tx.clone())))
    }

    fn transmit(&'a mut self) -> Option<Self::TxToken> {
        Some(AsyncTxToken(self.outgoing_tx.clone()))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = self.medium;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}
