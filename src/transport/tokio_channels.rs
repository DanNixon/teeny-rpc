use crate::{error, trace, warn};
use core::{marker::PhantomData, time::Duration};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::broadcast::{channel, Receiver, Sender};

pub struct TokioChannelTransport<M> {
    tx: Sender<u8>,
    rx: Receiver<u8>,
    _msg_type: PhantomData<M>,
}

impl<M> TokioChannelTransport<M> {
    pub fn new_pair(capacity: usize) -> (Self, Self) {
        let (mut controllers, mut targets) = Self::new_multidrop(capacity, 1, 1);
        (controllers.pop().unwrap(), targets.pop().unwrap())
    }

    pub fn new_multidrop(
        capacity: usize,
        num_controllers: usize,
        num_targets: usize,
    ) -> (Vec<Self>, Vec<Self>) {
        let (controller_out, _) = channel::<u8>(capacity);
        let (target_out, _) = channel::<u8>(capacity);

        let mut controllers = Vec::new();
        for _ in 0..num_controllers {
            let controller_in = target_out.subscribe();

            controllers.push(Self {
                tx: controller_out.clone(),
                rx: controller_in,
                _msg_type: PhantomData,
            });
        }

        let mut targets = Vec::new();
        for _ in 0..num_targets {
            let target_in = controller_out.subscribe();

            targets.push(Self {
                tx: target_out.clone(),
                rx: target_in,
                _msg_type: PhantomData,
            });
        }

        (controllers, targets)
    }

    pub(crate) fn transmit_raw(&mut self, data: &[u8]) -> Result<(), crate::Error> {
        for b in data {
            self.tx.send(*b).map_err(|_| crate::Error::TransportError)?;
        }

        Ok(())
    }
}

impl<M: Serialize + DeserializeOwned> super::Transport<M> for TokioChannelTransport<M> {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, crate::Error> {
        let mut count: usize = 0;

        loop {
            match tokio::time::timeout(timeout, self.rx.recv()).await {
                Ok(Ok(_)) => {
                    count = count.saturating_add(1);
                }
                Ok(Err(e)) => {
                    warn!("Channel error: {e}");
                    return Err(crate::Error::TransportError);
                }
                Err(_) => {
                    break;
                }
            }
        }

        Ok(count)
    }

    async fn receive_message(&mut self, timeout: Duration) -> Result<M, crate::Error> {
        let mut buffer = Vec::new();

        let start = tokio::time::Instant::now();

        loop {
            match tokio::time::timeout(Duration::from_millis(10), self.rx.recv()).await {
                Ok(Ok(b)) => {
                    buffer.push(b);

                    if buffer.last() == Some(&0u8) {
                        match postcard::from_bytes_cobs::<M>(buffer.as_mut_slice()) {
                            Ok(msg) => {
                                trace!("Received message");
                                buffer.clear();
                                return Ok(msg);
                            }
                            Err(_) => {
                                warn!(
                                    "Failed to decode message with {} bytes in buffer",
                                    buffer.len(),
                                );
                                buffer.clear();
                                return Err(crate::Error::TransportError);
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("Channel error: {e}");
                    return Err(crate::Error::TransportError);
                }
                Err(_) => {
                    let elapsed = tokio::time::Instant::now() - start;
                    if elapsed.as_micros() >= timeout.as_micros() {
                        return Err(crate::Error::Timeout);
                    }
                }
            }
        }
    }

    async fn transmit_message(&mut self, msg: M) -> Result<(), crate::Error> {
        let buffer = postcard::to_stdvec_cobs(&msg).map_err(|e| {
            error!("Serialize error: {e}");
            crate::Error::SerializeError
        })?;

        self.transmit_raw(&buffer)
    }
}
