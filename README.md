# `teeny-rpc`

[![Crates.io Version](https://img.shields.io/crates/v/teeny-rpc)](https://crates.io/crates/teeny-rpc)
[![docs.rs](https://img.shields.io/docsrs/teeny-rpc)](https://docs.rs/teeny-rpc/latest/teeny_rpc/)

A minimal async RPC library for Rust, with `std` and `no_std` clients and servers.

## Features

- Async request/response RPC over any transport implementing the `Transport` trait.
- Explicit request and response acknowledgements with configurable timeouts.
- Multiple clients on a single multidrop bus using 1-byte channel IDs.
- Built-in transports for serial ports, anything implementing `embedded-io-async` traits, and in-process Tokio channels (for testing).
- Supports both `std` and `no_std` (no `alloc` required).

## Limitations

- One request can be in flight per client/server endpoint at a time.
- No arbitrary payloads, the request and response types are fixed at compile time and must match for both sides of a channel client/server pair.
- `CHANNEL_ID` and `BUF_LEN` are compile-time constants. `BUF_LEN` is the maximum encoded frame size, including the header, payload, CRC, COBS overhead, and trailing delimiter.
- Acknowledgements detect missing or mismatched messages, but there are no automatic retries or exactly-once delivery. Retry at the application level if required.

## Examples

Two examples are included:

- [`examples/pc`](examples/pc) is a `std` client and server communicating over a serial port. From that directory:

  ```sh
  cargo run --release -- --port /dev/ttyACM0 server
  cargo run --release -- --port /dev/ttyACM0 client
  ```

- [`examples/pi-pico`](examples/pi-pico) is a `no_std` server for the RP2040, using UART0 on pins 0 and 1 at 115200 baud. From that directory, with a debug probe attached:

  ```sh
  cargo run --release
  ```

  The example's `.cargo/config.toml` uses `probe-rs` to flash and run the firmware.

## How it works

Each RPC call is a four-message exchange:

1. The client sends a `Request` with a new sequence number and waits for a
   `RequestAck` within the configured acknowledgement timeout.
2. The server receives the request, sends a `RequestAck`, and returns the
   request payload from `wait_for_request`.
3. The application handles the request and calls `send_response`. The server
   sends a `Response` and waits for a `ResponseAck`.
4. The client checks the channel and sequence number, sends a `ResponseAck`, and
   returns the response payload.

Sequence numbers are 16-bit and wrap around.
The acknowledgement timeout passed to `Client::new` and `Server::new` applies to acknowledgements, while the timeouts passed to `Client::call` and `Server::wait_for_request` apply to the request or response itself.

Frames use the following wire format:

| Field          | Size    | Description                                                        |
| -------------- | ------- | ------------------------------------------------------------------ |
| Channel ID     | 1 byte  | Logical channel/endpoint                                           |
| Message type   | 1 byte  | `Request` (0), `RequestAck` (1), `Response` (2), `ResponseAck` (3) |
| Sequence       | 2 bytes | Little-endian sequence number                                      |
| Payload length | 2 bytes | Little-endian payload length, excluding the header and CRC         |
| Payload        | N bytes | `postcard`-serialized request or response                          |
| CRC            | 2 bytes | CRC-16/IBM-3740 over the header and payload                        |

The complete frame, including the CRC, is COBS-encoded and terminated with a zero byte.
Acknowledgement messages have an empty payload.

## Transports

| Transport                | Feature  | Description                                                                                        |
| ------------------------ | -------- | -------------------------------------------------------------------------------------------------- |
| `SerialTransport`        | `std`    | Serial port transport using `tokio-serial`; construct with `new(port, baud)`                       |
| `TokioChannelTransport`  | `std`    | In-process multidrop bus using Tokio broadcast channels (added for testing, limited practical use) |
| `EioTransport`           | `no-std` | Transport for any `embedded-io-async` `Read + Write` device, such as a UART                        |

`TokioChannelTransport::new_pair` creates a single client/server pair, while `new_multidrop` creates multiple controllers and targets sharing one bus.

### Custom transports

Implement the `Transport` trait to use `teeny-rpc` over another link:

```rust
pub trait Transport {
    async fn flush(&mut self, timeout: Duration) -> Result<usize, Error>;
    async fn receive_frame(&mut self, timeout: Duration) -> Result<&mut [u8], Error>;
    async fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), Error>;
}
```

- `flush` should discard any pending incoming data and return the number of bytes discarded.
- `receive_frame` should wait up to the given timeout for one complete zero-delimited COBS frame and return it.
- `transmit_frame` should write the complete encoded frame, including the trailing zero delimiter.

## License

MIT.
