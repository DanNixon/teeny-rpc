use crate::{client::Client, server::Server, transport::tokio_channels::TokioChannelTransport};
use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[ctor::ctor]
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
    let mut client = Client::<_, Request, Response, 0x42>::new(t1, ack_timeout);
    let mut server = Server::<_, Request, Response, 0x42>::new(t2, ack_timeout);

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
    t2.transmit_raw(b"lol wtf?").unwrap();

    let ack_timeout = Duration::from_millis(100);
    let mut client = Client::<_, Request, Response, 0x42>::new(t1, ack_timeout);
    let mut server = Server::<_, Request, Response, 0x42>::new(t2, ack_timeout);

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
async fn multiple_instances_one_bus() {
    let (mut controller_transports, mut target_transports) =
        TokioChannelTransport::new_multidrop(256, 2, 2);

    let ack_timeout = Duration::from_millis(100);

    let mut client_1 = Client::<_, Request, Response, 0x01>::new(
        controller_transports.pop().unwrap(),
        ack_timeout,
    );
    let mut client_2 = Client::<_, Request, Response, 0x02>::new(
        controller_transports.pop().unwrap(),
        ack_timeout,
    );

    let mut server_1 =
        Server::<_, Request, Response, 0x01>::new(target_transports.pop().unwrap(), ack_timeout);
    let mut server_2 =
        Server::<_, Request, Response, 0x02>::new(target_transports.pop().unwrap(), ack_timeout);

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
