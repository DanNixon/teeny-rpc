use crate::Error;
use core::time::Duration;
use defmt::{trace, warn};
use embedded_io_async::{Read, Write};
use heapless::Vec;

pub struct EioTransport<T: Read + Write, const BUF_LEN: usize> {
    port: T,
    rx_buf: Vec<u8, BUF_LEN>,
}

impl<T: Read + Write, const BUF_LEN: usize> EioTransport<T, BUF_LEN> {
    pub fn new(port: T) -> Self {
        Self {
            port,
            rx_buf: Vec::new(),
        }
    }
}

impl<T: Read + Write, const BUF_LEN: usize> super::Transport for EioTransport<T, BUF_LEN> {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, crate::Error> {
        let mut count: usize = 0;

        loop {
            let mut b = [0u8];

            match embassy_time::with_timeout(
                embassy_time::Duration::from_micros(timeout.as_micros() as u64),
                self.port.read(&mut b),
            )
            .await
            {
                Ok(Ok(_)) => {
                    count = count.saturating_add(1);
                }
                Ok(Err(_)) => {
                    warn!("UART read fail");
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
        self.rx_buf.clear();

        let start = embassy_time::Instant::now();

        loop {
            let mut b = [0u8];

            match embassy_time::with_timeout(
                embassy_time::Duration::from_millis(10),
                self.port.read(&mut b),
            )
            .await
            {
                Ok(Ok(_)) => {
                    if self.rx_buf.extend_from_slice(&b).is_err() {
                        warn!("Failed to buffer frame with {} bytes", self.rx_buf.len(),);
                        self.rx_buf.clear();
                        return Err(Error::TransportError);
                    }

                    if b[0] == 0 {
                        trace!("Received frame");
                        return Ok(&mut self.rx_buf);
                    }
                }
                Ok(Err(_)) => {
                    warn!("UART read fail");
                }
                Err(_) => {
                    let elapsed = embassy_time::Instant::now() - start;
                    if elapsed.as_micros() as u128 >= timeout.as_micros() {
                        return Err(Error::Timeout);
                    }
                }
            }
        }
    }

    async fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), crate::Error> {
        self.port
            .write_all(frame)
            .await
            .map_err(|_| Error::TransportError)
    }
}
