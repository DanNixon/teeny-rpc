#![no_std]
#![no_main]

use core::time::Duration;
use defmt::warn;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    peripherals::UART0,
    uart::{BufferedInterruptHandler, BufferedUart, Config},
};
use panic_probe as _;
use portable_atomic as _;
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;
use teeny_rpc::{server::Server, transport::embedded::EioTransport};

bind_interrupts!(struct Irqs {
    UART0_IRQ  => BufferedInterruptHandler<UART0>;
});

#[derive(Clone, PartialEq, Serialize, Deserialize)]
enum Request {
    Ping(u32),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
enum Response {
    Ping(u32),
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    static TX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; 16])[..];
    static RX_BUF: StaticCell<[u8; 16]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; 16])[..];

    let mut config = Config::default();
    config.baudrate = 115_200;

    let uart = BufferedUart::new(p.UART0, p.PIN_0, p.PIN_1, Irqs, tx_buf, rx_buf, config);

    let transport = EioTransport::<_, 512>::new(uart);
    let mut server = Server::<_, Request, Response>::new(transport, Duration::from_millis(100));

    loop {
        match server.wait_for_request(Duration::from_secs(5)).await {
            Ok(request) => {
                let response = match request {
                    Request::Ping(i) => Response::Ping(i),
                };
                if let Err(e) = server.send_response(response).await {
                    warn!("Server failed sending response: {}", e);
                }
            }
            Err(e) => {
                warn!("Server failed waiting for request: {}", e);
            }
        }
    }
}
