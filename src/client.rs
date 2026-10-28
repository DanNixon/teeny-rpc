use crate::{debug, trace, transport::Transport, warn, wire, Error};
use core::{marker::PhantomData, time::Duration};
use serde::{de::DeserializeOwned, Serialize};

pub struct Client<
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
    seq: u16,
    _request: PhantomData<REQ>,
    _response: PhantomData<RESP>,
}

impl<
        T,
        REQ: Serialize + DeserializeOwned,
        RESP: Serialize + DeserializeOwned,
        const CHANNEL_ID: u8,
        const BUF_LEN: usize,
    > Client<T, REQ, RESP, CHANNEL_ID, BUF_LEN>
where
    T: Transport,
{
    pub fn new(transport: T, ack_timeout: Duration) -> Self {
        Self {
            transport,
            ack_timeout,
            seq: 0,
            _request: PhantomData,
            _response: PhantomData,
        }
    }

    pub async fn call(&mut self, request: REQ, timeout: Duration) -> Result<RESP, crate::Error> {
        let flushed_bytes = self.transport.flush(Duration::from_millis(5)).await?;
        debug!("Flushed {} bytes prior to request", flushed_bytes);

        self.seq = self.seq.wrapping_add(1);
        debug!("New request {}", self.seq);

        // Send the request
        trace!("Sending request {}", self.seq);
        wire::transmit_request::<BUF_LEN, _, REQ>(
            &mut self.transport,
            CHANNEL_ID,
            self.seq,
            &request,
        )
        .await?;

        // Receive the request acknowledgement
        match wire::receive_request_ack(&mut self.transport, self.ack_timeout).await {
            Ok((channel_id, seq)) => {
                if channel_id != CHANNEL_ID {
                    return Err(Error::IncorrectChannel {
                        expected: CHANNEL_ID,
                        actual: channel_id,
                    });
                }

                if seq != self.seq {
                    return Err(Error::IncorrectSequenceNumber {
                        expected: self.seq,
                        actual: seq,
                    });
                }

                trace!("Received ack for request {}", self.seq);
            }
            Err(Error::Timeout) => {
                warn!("Timeout waiting for ack for request {}", self.seq);
                return Err(Error::NoAck);
            }
            Err(Error::IncorrectMessageType) => {
                warn!("Incorrect message type for ack for request {}", self.seq);
                return Err(Error::IncorrectMessageType);
            }
            Err(e) => {
                warn!("Error waiting for ack for request {}: {}", self.seq, e);
                return Err(Error::NoAck);
            }
        }

        // Receive the response
        trace!("Waiting for response for request {}", self.seq);
        let (channel_id, seq, payload) =
            wire::receive_response::<_, RESP>(&mut self.transport, timeout).await?;

        if channel_id != CHANNEL_ID {
            return Err(Error::IncorrectChannel {
                expected: CHANNEL_ID,
                actual: channel_id,
            });
        }

        if seq != self.seq {
            return Err(Error::IncorrectSequenceNumber {
                expected: self.seq,
                actual: seq,
            });
        }

        // Send the response acknowledgement
        trace!("Sending ack for response {}", self.seq);
        wire::transmit_response_ack::<BUF_LEN, _>(&mut self.transport, CHANNEL_ID, self.seq)
            .await?;

        debug!("Request {} complete", self.seq);
        Ok(payload)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        client::Client,
        test::{
            receive_request, receive_response_ack, transmit_request_ack, transmit_response,
            Request, Response, BUF_LEN, TEST_TIMEOUT,
        },
        transport::TokioChannelTransport,
        Error,
    };
    use core::time::Duration;

    const ACK_TIMEOUT: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn basic() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            let (channel_id, seq, request) = receive_request(&mut t2, TEST_TIMEOUT).await.unwrap();
            assert_eq!(channel_id, 0x42);
            assert_eq!(seq, 1);
            assert_eq!(request, Request::Ping(42));

            transmit_request_ack(&mut t2, 0x42, 1).await.unwrap();

            transmit_response(&mut t2, 0x42, 1, &Response::Ping(42))
                .await
                .unwrap();

            let (channel_id, seq) = receive_response_ack(&mut t2, TEST_TIMEOUT).await.unwrap();
            assert_eq!(channel_id, 0x42);
            assert_eq!(seq, 1);
        };

        let run_client = async move {
            let response = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await
                .unwrap();
            assert_eq!(response, Response::Ping(42));
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn no_ack() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            tokio::time::sleep(ACK_TIMEOUT).await;

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Err(Error::NoAck));
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn no_response() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x42, 1).await.unwrap();

            // Wait for the request to timeout
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Err(Error::Timeout));
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn wrong_channel_ack() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x43, 1).await.unwrap();

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(
                result,
                Err(Error::IncorrectChannel {
                    expected: 0x42,
                    actual: 0x43
                })
            );
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn wrong_channel_response() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x42, 1).await.unwrap();

            transmit_response(&mut t2, 0x43, 1, &Response::Ping(42))
                .await
                .unwrap();

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(
                result,
                Err(Error::IncorrectChannel {
                    expected: 0x42,
                    actual: 0x43
                })
            );
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn recover_after_no_ack() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            // No ack
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            tokio::time::sleep(ACK_TIMEOUT).await;

            // Normal
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 2, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x42, 2).await.unwrap();

            transmit_response(&mut t2, 0x42, 2, &Response::Ping(42))
                .await
                .unwrap();

            assert_eq!(
                receive_response_ack(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            // No ack
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Err(Error::NoAck));

            // Normal
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Ok(Response::Ping(42)));
        };

        tokio::join!(run_server, run_client);
    }

    #[tokio::test]
    async fn recover_after_no_response() {
        let (t1, mut t2) = TokioChannelTransport::new_pair(256);

        let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ACK_TIMEOUT);

        let run_server = async move {
            // No response
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 1, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x42, 1).await.unwrap();

            // Wait for the request to timeout
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Normal
            assert_eq!(
                receive_request(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 2, Request::Ping(42)))
            );

            transmit_request_ack(&mut t2, 0x42, 2).await.unwrap();

            transmit_response(&mut t2, 0x42, 2, &Response::Ping(42))
                .await
                .unwrap();

            assert_eq!(
                receive_response_ack(&mut t2, TEST_TIMEOUT).await,
                Ok((0x42, 2))
            );

            // Exit when client goes away
            assert_eq!(
                receive_request(&mut t2, Duration::from_secs(30)).await,
                Err(Error::TransportError)
            );
        };

        let run_client = async move {
            // No response
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Err(Error::Timeout));

            // Normal
            let result = client
                .call(Request::Ping(42), Duration::from_millis(500))
                .await;
            assert_eq!(result, Ok(Response::Ping(42)));
        };

        tokio::join!(run_server, run_client);
    }
}
