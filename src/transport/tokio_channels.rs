use crate::{trace, warn, Error};
use core::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{channel, Receiver, Sender};

pub struct TokioChannelTransport {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    rx_buf: Vec<u8>,
}

impl TokioChannelTransport {
    /// Create a connected pair of transports.
    ///
    /// `capacity` is the number of buffered frames (not bytes) in each
    /// direction of the channel pair.
    pub fn new_pair(capacity: usize) -> (Self, Self) {
        let (mut controllers, mut targets) = Self::new_multidrop(capacity, 1, 1);
        (controllers.pop().unwrap(), targets.pop().unwrap())
    }

    /// Create a set of transports sharing a multidrop bus.
    ///
    /// `capacity` is the number of buffered frames (not bytes) in each
    /// direction of the bus.
    pub fn new_multidrop(
        capacity: usize,
        num_controllers: usize,
        num_targets: usize,
    ) -> (Vec<Self>, Vec<Self>) {
        let (controller_out, _) = channel::<Vec<u8>>(capacity);
        let (target_out, _) = channel::<Vec<u8>>(capacity);

        let mut controllers = Vec::new();
        for _ in 0..num_controllers {
            let controller_in = target_out.subscribe();

            controllers.push(Self {
                tx: controller_out.clone(),
                rx: controller_in,
                rx_buf: Vec::new(),
            });
        }

        let mut targets = Vec::new();
        for _ in 0..num_targets {
            let target_in = controller_out.subscribe();

            targets.push(Self {
                tx: target_out.clone(),
                rx: target_in,
                rx_buf: Vec::new(),
            });
        }

        (controllers, targets)
    }
}

impl super::Transport for TokioChannelTransport {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, crate::Error> {
        let mut count: usize = 0;

        loop {
            match tokio::time::timeout(timeout, self.rx.recv()).await {
                Ok(Ok(frame)) => {
                    count = count.saturating_add(frame.len());
                }
                Ok(Err(RecvError::Lagged(n))) => {
                    warn!("Channel lagged, {} frames lost", n);
                }
                Ok(Err(e)) => {
                    warn!("Channel error: {e}");
                    return Err(Error::TransportError);
                }
                Err(_) => {
                    break;
                }
            }
        }

        Ok(count)
    }

    async fn receive_frame(&mut self, timeout: Duration) -> Result<&mut [u8], crate::Error> {
        let start = tokio::time::Instant::now();

        loop {
            let remaining = timeout.saturating_sub(start.elapsed());
            match tokio::time::timeout(remaining, self.rx.recv()).await {
                Ok(Ok(frame)) => {
                    self.rx_buf = frame;
                    trace!("Received frame");
                    return Ok(&mut self.rx_buf);
                }
                Ok(Err(RecvError::Lagged(n))) => {
                    warn!("Channel lagged, {} frames lost", n);
                    // Frames lost due to lag are self contained, the frame
                    // being waited for may still arrive
                }
                Ok(Err(e)) => {
                    warn!("Channel error: {e}");
                    return Err(Error::TransportError);
                }
                Err(_) => return Err(Error::Timeout),
            }
        }
    }

    async fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), crate::Error> {
        self.tx
            .send(frame.to_vec())
            .map(|_| ())
            .map_err(|_| Error::TransportError)
    }
}
