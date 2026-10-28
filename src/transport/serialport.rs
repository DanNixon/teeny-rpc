use crate::Error;
use core::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

pub struct SerialTransport {
    reader: BufReader<ReadHalf<SerialStream>>,
    writer: WriteHalf<SerialStream>,
    rx_buf: Vec<u8>,
}

impl SerialTransport {
    pub fn new(port: &str, baud: u32) -> Result<Self, crate::Error> {
        let port = tokio_serial::new(port, baud)
            .open_native_async()
            .map_err(|e| {
                error!("Failed to open serial port {} with error {}", port, e);
                Error::TransportError
            })?;

        let (reader, writer) = tokio::io::split(port);
        let reader = BufReader::new(reader);

        Ok(Self {
            reader,
            writer,
            rx_buf: Vec::new(),
        })
    }
}

impl super::Transport for SerialTransport {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, crate::Error> {
        let mut count: usize = 0;

        loop {
            match tokio::time::timeout(timeout, self.reader.read_u8()).await {
                Ok(Ok(_)) => {
                    count = count.saturating_add(1);
                }
                Ok(Err(_)) => {
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

        match tokio::time::timeout(timeout, self.reader.read_until(b'\0', &mut self.rx_buf)).await {
            Ok(Ok(0)) => {
                error!("No data received");
                Err(Error::TransportError)
            }
            Ok(Ok(_)) => Ok(&mut self.rx_buf),
            Ok(Err(e)) => {
                error!("Serial error: {}", e);
                Err(Error::TransportError)
            }
            Err(_) => Err(Error::Timeout),
        }
    }

    async fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), crate::Error> {
        self.writer.write_all(frame).await.map_err(|e| {
            error!("Failed to write to serial port with error {}", e);
            Error::TransportError
        })
    }
}
