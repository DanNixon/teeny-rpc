use crate::Error;
use core::time::Duration;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{de::DeserializeOwned, Serialize};

const HEADER_LEN: usize = 6;

const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_IBM_3740);
const CRC_LEN: usize = 2;

#[derive(Copy, Clone, Debug, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
enum MessageType {
    Request = 0,
    RequestAck = 1,
    Response = 2,
    ResponseAck = 3,
}

pub(crate) type FrameBuffer<const BUF_LEN: usize> = heapless::Vec<u8, BUF_LEN>;

fn header(
    channel_id: u8,
    message_type: MessageType,
    sequence: u16,
    payload_len: usize,
) -> Result<[u8; HEADER_LEN], Error> {
    let payload_len = u16::try_from(payload_len).map_err(|_| Error::FrameTooLong)?;

    let mut header = [0u8; HEADER_LEN];
    header[0] = channel_id;
    header[1] = message_type.into();
    header[2..4].copy_from_slice(&sequence.to_le_bytes());
    header[4..6].copy_from_slice(&payload_len.to_le_bytes());
    Ok(header)
}

fn serialize_payload<P: Serialize, const BUF_LEN: usize>(
    payload: &P,
) -> Result<heapless::Vec<u8, BUF_LEN>, Error> {
    let mut buffer = heapless::Vec::<u8, BUF_LEN>::new();
    buffer.resize(BUF_LEN, 0).map_err(|_| Error::FrameTooLong)?;
    let payload = postcard::to_slice(payload, &mut buffer).map_err(|_| Error::SerializeError)?;
    let len = payload.len();
    buffer.truncate(len);
    Ok(buffer)
}

fn encode<const BUF_LEN: usize>(
    channel_id: u8,
    message_type: MessageType,
    sequence: u16,
    payload: &[u8],
) -> Result<FrameBuffer<BUF_LEN>, Error> {
    let header = header(channel_id, message_type, sequence, payload.len())?;

    let mut raw = heapless::Vec::<u8, BUF_LEN>::new();
    raw.extend_from_slice(&header)
        .map_err(|_| Error::FrameTooLong)?;
    raw.extend_from_slice(payload)
        .map_err(|_| Error::FrameTooLong)?;
    raw.extend_from_slice(&CRC.checksum(&raw).to_le_bytes())
        .map_err(|_| Error::FrameTooLong)?;

    let mut frame = heapless::Vec::<u8, BUF_LEN>::new();
    frame.resize(BUF_LEN, 0).map_err(|_| Error::FrameTooLong)?;
    let mut encoded_len =
        cobs::try_encode(&raw, &mut frame[..]).map_err(|_| Error::FrameTooLong)?;
    frame.truncate(encoded_len);
    frame.push(0).map_err(|_| Error::FrameTooLong)?;
    encoded_len += 1;
    debug_assert_eq!(frame.len(), encoded_len);

    Ok(frame)
}

fn encode_payload<P: Serialize, const BUF_LEN: usize>(
    channel_id: u8,
    message_type: MessageType,
    sequence: u16,
    payload: &P,
) -> Result<FrameBuffer<BUF_LEN>, Error> {
    let payload = serialize_payload::<P, BUF_LEN>(payload)?;
    encode::<BUF_LEN>(channel_id, message_type, sequence, &payload)
}

fn encode_empty<const BUF_LEN: usize>(
    channel_id: u8,
    message_type: MessageType,
    sequence: u16,
) -> Result<FrameBuffer<BUF_LEN>, Error> {
    encode::<BUF_LEN>(channel_id, message_type, sequence, &[])
}

fn decode_frame(frame: &mut [u8], expected_type: MessageType) -> Result<(u8, u16, &[u8]), Error> {
    let decoded_len = cobs::decode_in_place(frame).map_err(|_| Error::DeserializeError)?;
    let frame = &frame[..decoded_len];

    if frame.len() < HEADER_LEN + CRC_LEN {
        return Err(Error::DeserializeError);
    }

    let payload_len = u16::from_le_bytes([frame[4], frame[5]]) as usize;
    if frame.len() != HEADER_LEN + payload_len + CRC_LEN {
        return Err(Error::DeserializeError);
    }

    let (data, received_crc) = frame.split_at(frame.len() - CRC_LEN);
    let received_crc = u16::from_le_bytes([received_crc[0], received_crc[1]]);
    if CRC.checksum(data) != received_crc {
        return Err(Error::CrcError);
    }

    let channel_id = frame[0];
    let message_type = MessageType::try_from(frame[1]).map_err(|_| Error::IncorrectMessageType)?;
    let sequence = u16::from_le_bytes([frame[2], frame[3]]);
    let payload = &frame[HEADER_LEN..HEADER_LEN + payload_len];

    if message_type != expected_type {
        return Err(Error::IncorrectMessageType);
    }

    if matches!(
        message_type,
        MessageType::RequestAck | MessageType::ResponseAck
    ) && payload_len != 0
    {
        return Err(Error::DeserializeError);
    }

    Ok((channel_id, sequence, payload))
}

fn decode_payload<P: DeserializeOwned>(
    frame: &mut [u8],
    expected_type: MessageType,
) -> Result<(u8, u16, P), Error> {
    let (channel_id, sequence, payload) = decode_frame(frame, expected_type)?;
    let payload = postcard::from_bytes(payload).map_err(|_| Error::DeserializeError)?;
    Ok((channel_id, sequence, payload))
}

fn decode_empty(frame: &mut [u8], expected_type: MessageType) -> Result<(u8, u16), Error> {
    let (channel_id, sequence, _) = decode_frame(frame, expected_type)?;
    Ok((channel_id, sequence))
}

pub(crate) async fn transmit_request<const BUF_LEN: usize, T, REQ>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
    payload: &REQ,
) -> Result<(), Error>
where
    T: crate::transport::Transport,
    REQ: Serialize,
{
    let frame =
        encode_payload::<REQ, BUF_LEN>(channel_id, MessageType::Request, sequence, payload)?;
    transport.transmit_frame(&frame).await
}

pub(crate) async fn receive_request<T, REQ>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16, REQ), Error>
where
    T: crate::transport::Transport,
    REQ: DeserializeOwned,
{
    let frame = transport.receive_frame(timeout).await?;
    decode_payload(frame, MessageType::Request)
}

pub(crate) async fn transmit_response<const BUF_LEN: usize, T, RESP>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
    payload: &RESP,
) -> Result<(), Error>
where
    T: crate::transport::Transport,
    RESP: Serialize,
{
    let frame =
        encode_payload::<RESP, BUF_LEN>(channel_id, MessageType::Response, sequence, payload)?;
    transport.transmit_frame(&frame).await
}

pub(crate) async fn receive_response<T, RESP>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16, RESP), Error>
where
    T: crate::transport::Transport,
    RESP: DeserializeOwned,
{
    let frame = transport.receive_frame(timeout).await?;
    decode_payload(frame, MessageType::Response)
}

pub(crate) async fn transmit_request_ack<const BUF_LEN: usize, T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
) -> Result<(), Error>
where
    T: crate::transport::Transport,
{
    let frame = encode_empty::<BUF_LEN>(channel_id, MessageType::RequestAck, sequence)?;
    transport.transmit_frame(&frame).await
}

pub(crate) async fn receive_request_ack<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16), Error>
where
    T: crate::transport::Transport,
{
    let frame = transport.receive_frame(timeout).await?;
    decode_empty(frame, MessageType::RequestAck)
}

pub(crate) async fn transmit_response_ack<const BUF_LEN: usize, T>(
    transport: &mut T,
    channel_id: u8,
    sequence: u16,
) -> Result<(), Error>
where
    T: crate::transport::Transport,
{
    let frame = encode_empty::<BUF_LEN>(channel_id, MessageType::ResponseAck, sequence)?;
    transport.transmit_frame(&frame).await
}

pub(crate) async fn receive_response_ack<T>(
    transport: &mut T,
    timeout: Duration,
) -> Result<(u8, u16), Error>
where
    T: crate::transport::Transport,
{
    let frame = transport.receive_frame(timeout).await?;
    decode_empty(frame, MessageType::ResponseAck)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test::{Request, Response, BUF_LEN};

    fn decode_to_raw(mut frame: FrameBuffer<BUF_LEN>) -> FrameBuffer<BUF_LEN> {
        let len = cobs::decode_in_place(&mut frame).unwrap();
        frame.truncate(len);
        frame
    }

    #[test]
    fn round_trip_all_message_kinds() {
        let mut frame =
            encode_payload::<Request, BUF_LEN>(0x42, MessageType::Request, 1, &Request::Ping(42))
                .unwrap();
        assert_eq!(
            decode_payload::<Request>(&mut frame, MessageType::Request).unwrap(),
            (0x42, 1, Request::Ping(42))
        );

        let mut frame = encode_empty::<BUF_LEN>(0x42, MessageType::RequestAck, 2).unwrap();
        assert_eq!(
            decode_empty(&mut frame, MessageType::RequestAck).unwrap(),
            (0x42, 2)
        );

        let mut frame = encode_payload::<Response, BUF_LEN>(
            0x42,
            MessageType::Response,
            3,
            &Response::Ping(69),
        )
        .unwrap();
        assert_eq!(
            decode_payload::<Response>(&mut frame, MessageType::Response).unwrap(),
            (0x42, 3, Response::Ping(69))
        );

        let mut frame = encode_empty::<BUF_LEN>(0x42, MessageType::ResponseAck, 4).unwrap();
        assert_eq!(
            decode_empty(&mut frame, MessageType::ResponseAck).unwrap(),
            (0x42, 4)
        );
    }

    #[test]
    fn header_layout_and_crc() {
        let frame = encode_payload::<Request, BUF_LEN>(
            0x42,
            MessageType::Request,
            0x1234,
            &Request::Ping(42),
        )
        .unwrap();
        let raw = decode_to_raw(frame);

        assert_eq!(raw[0], 0x42);
        assert_eq!(raw[1], u8::from(MessageType::Request));
        assert_eq!(u16::from_le_bytes([raw[2], raw[3]]), 0x1234);
        assert_eq!(
            u16::from_le_bytes([raw[4], raw[5]]) as usize,
            raw.len() - HEADER_LEN - CRC_LEN
        );
        assert_eq!(
            CRC.checksum(&raw[..raw.len() - CRC_LEN]),
            u16::from_le_bytes([raw[raw.len() - 2], raw[raw.len() - 1]])
        );
    }
}
