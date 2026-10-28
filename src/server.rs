use crate::{debug, trace, transport::Transport, warn, wire, Error};
use core::{marker::PhantomData, time::Duration};
use serde::{de::DeserializeOwned, Serialize};

pub struct Server<
    T,
    REQ: Serialize + DeserializeOwned,
    RESP: Serialize + DeserializeOwned,
    const CHANNEL_ID: u8,
    const BUF_LEN: usize,
> where
    T: Transport,
{
    transport: T,
    ack_timeout: Duration,
    active_rpc: Option<u16>,
    _request: PhantomData<REQ>,
    _response: PhantomData<RESP>,
}

impl<
        T,
        REQ: Serialize + DeserializeOwned,
        RESP: Serialize + DeserializeOwned,
        const CHANNEL_ID: u8,
        const BUF_LEN: usize,
    > Server<T, REQ, RESP, CHANNEL_ID, BUF_LEN>
where
    T: Transport,
{
    pub fn new(transport: T, ack_timeout: Duration) -> Self {
        Self {
            transport,
            ack_timeout,
            active_rpc: None,
            _request: PhantomData,
            _response: PhantomData,
        }
    }

    pub async fn wait_for_request(&mut self, timeout: Duration) -> Result<REQ, crate::Error> {
        if self.active_rpc.is_some() {
            return Err(Error::RequestAlreadyInProgress);
        }

        let (channel_id, seq, payload) =
            wire::receive_request::<_, REQ>(&mut self.transport, timeout).await?;

        if channel_id != CHANNEL_ID {
            return Err(Error::IncorrectChannel {
                expected: CHANNEL_ID,
                actual: channel_id,
            });
        }

        debug!("Received request {}", seq);

        trace!("Sending ack for request {}", seq);
        wire::transmit_request_ack::<BUF_LEN, _>(&mut self.transport, channel_id, seq).await?;

        self.active_rpc = Some(seq);
        Ok(payload)
    }

    pub async fn send_response(&mut self, response: RESP) -> Result<(), crate::Error> {
        // Leave no active RPC after any attempt to send a response (cannot recover it if
        // send_response fails anyway)
        if let Some(seq) = self.active_rpc.take() {
            // Send the response
            trace!("Sending response {}", seq);
            wire::transmit_response::<BUF_LEN, _, RESP>(
                &mut self.transport,
                CHANNEL_ID,
                seq,
                &response,
            )
            .await?;

            // Receive the response acknowledgement
            match wire::receive_response_ack(&mut self.transport, self.ack_timeout).await {
                Ok((channel_id, ack_seq)) => {
                    if channel_id != CHANNEL_ID {
                        return Err(Error::IncorrectChannel {
                            expected: CHANNEL_ID,
                            actual: channel_id,
                        });
                    }
                    if ack_seq != seq {
                        return Err(Error::IncorrectSequenceNumber {
                            expected: seq,
                            actual: ack_seq,
                        });
                    }

                    trace!("Received ack for response {}", seq);

                    debug!("Request {} complete", seq);
                    Ok(())
                }
                Err(Error::Timeout) => {
                    warn!("Timeout waiting for ack for response {}", seq);
                    Err(Error::NoAck)
                }
                Err(Error::IncorrectMessageType) => {
                    warn!("Incorrect message type for ack for response {}", seq);
                    Err(Error::IncorrectMessageType)
                }
                Err(e) => {
                    warn!("Error waiting for ack for response {}: {}", seq, e);
                    Err(Error::NoAck)
                }
            }
        } else {
            Err(Error::NoRequestInProgress)
        }
    }

    pub fn ignore_active_request(&mut self) -> Result<(), crate::Error> {
        match self.active_rpc.take() {
            Some(seq) => {
                debug!("Ignoring request {}", seq);
                Ok(())
            }
            None => Err(Error::NoRequestInProgress),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        server::Server,
        test::{
            receive_request_ack, receive_response, transmit_request, transmit_response_ack,
            Request, Response, BUF_LEN, TEST_TIMEOUT,
        },
        transport::TokioChannelTransport,
        Error,
    };
    use core::time::Duration;

    const ACK_TIMEOUT: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn basic() {
        let (mut t1, t2) = TokioChannelTransport::new_pair(256);

        let mut server = Server::<_, Request, Response, 0x42, BUF_LEN>::new(t2, ACK_TIMEOUT);

        let run_server = {
            async move {
                loop {
                    let request = match server.wait_for_request(Duration::from_secs(5)).await {
                        Ok(request) => request,
                        Err(Error::TransportError) => {
                            // Exit when the client goes away
                            break;
                        }
                        Err(e) => {
                            panic!("Server error: {e}");
                        }
                    };

                    server
                        .send_response(match request {
                            Request::Ping(i) => Response::Ping(i),
                        })
                        .await
                        .unwrap();
                }
            }
        };

        let run_client = async move {
            transmit_request(&mut t1, 0x42, 2, &Request::Ping(69))
                .await
                .unwrap();

            assert_eq!(
                receive_request_ack(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            assert_eq!(
                receive_response(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2, Response::Ping(69)))
            );

            transmit_response_ack(&mut t1, 0x42, 2).await.unwrap();
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn no_ack() {
        let (mut t1, t2) = TokioChannelTransport::new_pair(256);

        let mut server = Server::<_, Request, Response, 0x42, BUF_LEN>::new(t2, ACK_TIMEOUT);

        let run_server = {
            async move {
                loop {
                    let request = match server.wait_for_request(Duration::from_secs(5)).await {
                        Ok(request) => request,
                        Err(Error::TransportError) => {
                            // Exit when the client goes away
                            break;
                        }
                        Err(e) => {
                            panic!("Server error: {e}");
                        }
                    };

                    let result = server
                        .send_response(match request {
                            Request::Ping(i) => Response::Ping(i),
                        })
                        .await;
                    assert_eq!(result, Err(Error::NoAck));
                }
            }
        };

        let run_client = async move {
            transmit_request(&mut t1, 0x42, 2, &Request::Ping(69))
                .await
                .unwrap();

            assert_eq!(
                receive_request_ack(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            assert_eq!(
                receive_response(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2, Response::Ping(69)))
            );

            tokio::time::sleep(ACK_TIMEOUT).await;

            // Wait for the request to timeout
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn recover_after_no_ack() {
        let (mut t1, t2) = TokioChannelTransport::new_pair(256);

        let mut server = Server::<_, Request, Response, 0x42, BUF_LEN>::new(t2, ACK_TIMEOUT);

        let run_server = {
            async move {
                loop {
                    let request = match server.wait_for_request(Duration::from_secs(5)).await {
                        Ok(request) => request,
                        Err(Error::TransportError) => {
                            // Exit when the client goes away
                            break;
                        }
                        Err(e) => {
                            panic!("Server error: {e}");
                        }
                    };

                    let _ = server
                        .send_response(match request {
                            Request::Ping(i) => Response::Ping(i),
                        })
                        .await;
                }
            }
        };

        let run_client = async move {
            transmit_request(&mut t1, 0x42, 2, &Request::Ping(69))
                .await
                .unwrap();

            assert_eq!(
                receive_request_ack(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            assert_eq!(
                receive_response(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2, Response::Ping(69)))
            );

            tokio::time::sleep(ACK_TIMEOUT).await;

            // Wait for the request to timeout
            tokio::time::sleep(Duration::from_millis(20)).await;

            transmit_request(&mut t1, 0x42, 2, &Request::Ping(69))
                .await
                .unwrap();

            assert_eq!(
                receive_request_ack(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            assert_eq!(
                receive_response(&mut t1, TEST_TIMEOUT).await,
                Ok((0x42, 2, Response::Ping(69)))
            );

            transmit_response_ack(&mut t1, 0x42, 2).await.unwrap();
        };

        tokio::join!(run_server, run_client);
    }
}
