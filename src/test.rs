use crate::{
    client::Client,
    server::Server,
    transport::{TokioChannelTransport, Transport},
    wire, Error,
};
use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const BUF_LEN: usize = 512;

pub(crate) async fn transmit_request<T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
    request: &Request,
) -> Result<(), Error>
where
    T: Transport,
{
    wire::transmit_request::<BUF_LEN, _, _>(transport, channel_id, sequence, request).await
}

pub(crate) async fn receive_request<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16, Request), Error>
where
    T: Transport,
{
    wire::receive_request(transport, timeout).await
}

pub(crate) async fn transmit_request_ack<T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
) -> Result<(), Error>
where
    T: Transport,
{
    wire::transmit_request_ack::<BUF_LEN, _>(transport, channel_id, sequence).await
}

pub(crate) async fn receive_request_ack<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16), Error>
where
    T: Transport,
{
    wire::receive_request_ack(transport, timeout).await
}

pub(crate) async fn transmit_response<T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
    response: &Response,
) -> Result<(), Error>
where
    T: Transport,
{
    wire::transmit_response::<BUF_LEN, _, _>(transport, channel_id, sequence, response).await
}

pub(crate) async fn receive_response<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16, Response), Error>
where
    T: Transport,
{
    wire::receive_response(transport, timeout).await
}

pub(crate) async fn transmit_response_ack<T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
) -> Result<(), Error>
where
    T: Transport,
{
    wire::transmit_response_ack::<BUF_LEN, _>(transport, channel_id, sequence).await
}

pub(crate) async fn receive_response_ack<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16), Error>
where
    T: Transport,
{
    wire::receive_response_ack(transport, timeout).await
}

#[ctor::ctor(unsafe)]
fn init_test_logging() {
    env_logger::init();
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub(crate) enum Request {
    Ping(i32),
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub(crate) enum Response {
    Ping(i32),
}

#[tokio::test]
async fn basic_rpc_transaction() {
    let (t1, t2) = TokioChannelTransport::new_pair(256);

    let ack_timeout = Duration::from_millis(100);
    let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ack_timeout);
    let mut server = Server::<_, Request, Response, 0x42, BUF_LEN>::new(t2, ack_timeout);

    let done = Arc::new(AtomicBool::new(false));

    let run_server = {
        let done = done.clone();
        async move {
            loop {
                if done.load(Ordering::Relaxed) {
                    break;
                }

                let request = server
                    .wait_for_request(Duration::from_secs(5))
                    .await
                    .unwrap();
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
        let response = client
            .call(Request::Ping(42), Duration::from_millis(500))
            .await
            .unwrap();
        assert_eq!(response, Response::Ping(42));
        done.store(true, Ordering::Relaxed)
    };

    tokio::join!(run_server, run_client);
}

#[tokio::test]
async fn recover_from_out_of_sync() {
    let (t1, mut t2) = TokioChannelTransport::new_pair(256);

    // Send some garbage data, which could be half a message that was interrupted
    t2.transmit_frame(b"lol wtf?\0").await.unwrap();

    let ack_timeout = Duration::from_millis(100);
    let mut client = Client::<_, Request, Response, 0x42, BUF_LEN>::new(t1, ack_timeout);
    let mut server = Server::<_, Request, Response, 0x42, BUF_LEN>::new(t2, ack_timeout);

    let done = Arc::new(AtomicBool::new(false));

    let run_server = {
        let done = done.clone();
        async move {
            let mut bad_frame_seen = false;

            loop {
                if done.load(Ordering::Relaxed) {
                    break;
                }

                let request = match server.wait_for_request(Duration::from_secs(5)).await {
                    Ok(request) => request,
                    Err(_) if !bad_frame_seen => {
                        bad_frame_seen = true;
                        continue;
                    }
                    Err(e) => panic!("Server error: {e}"),
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
        let response = client
            .call(Request::Ping(42), Duration::from_millis(500))
            .await
            .unwrap();
        assert_eq!(response, Response::Ping(42));
        done.store(true, Ordering::Relaxed)
    };

    tokio::join!(run_server, run_client);
}

#[tokio::test]
async fn multiple_instances_one_bus() {
    let (mut controller_transports, mut target_transports) =
        TokioChannelTransport::new_multidrop(256, 2, 2);

    let ack_timeout = Duration::from_millis(100);

    let mut client_1 = Client::<_, Request, Response, 0x01, BUF_LEN>::new(
        controller_transports.pop().unwrap(),
        ack_timeout,
    );
    let mut client_2 = Client::<_, Request, Response, 0x02, BUF_LEN>::new(
        controller_transports.pop().unwrap(),
        ack_timeout,
    );

    let mut server_1 = Server::<_, Request, Response, 0x01, BUF_LEN>::new(
        target_transports.pop().unwrap(),
        ack_timeout,
    );
    let mut server_2 = Server::<_, Request, Response, 0x02, BUF_LEN>::new(
        target_transports.pop().unwrap(),
        ack_timeout,
    );

    let done = Arc::new(AtomicBool::new(false));

    let run_server_1 = {
        let done = done.clone();
        async move {
            loop {
                if done.load(Ordering::Relaxed) {
                    break;
                }

                if let Ok(request) = server_1.wait_for_request(Duration::from_secs(5)).await {
                    server_1
                        .send_response(match request {
                            Request::Ping(i) => Response::Ping(i - 1),
                        })
                        .await
                        .unwrap();
                }
            }
        }
    };

    let run_server_2 = {
        let done = done.clone();
        async move {
            loop {
                if done.load(Ordering::Relaxed) {
                    break;
                }

                if let Ok(request) = server_2.wait_for_request(Duration::from_secs(5)).await {
                    server_2
                        .send_response(match request {
                            Request::Ping(i) => Response::Ping(i + 1),
                        })
                        .await
                        .unwrap();
                }
            }
        }
    };

    let run_clients = async move {
        let response = client_1
            .call(Request::Ping(42), Duration::from_millis(500))
            .await
            .unwrap();
        assert_eq!(response, Response::Ping(42 - 1));

        let response = client_2
            .call(Request::Ping(42), Duration::from_millis(500))
            .await
            .unwrap();
        assert_eq!(response, Response::Ping(42 + 1));

        done.store(true, Ordering::Relaxed)
    };

    tokio::join!(run_server_1, run_server_2, run_clients);
}
