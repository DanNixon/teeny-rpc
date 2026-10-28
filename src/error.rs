#[derive(thiserror::Error, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "no-std", derive(defmt::Format))]
pub enum Error {
    #[error("An RPC request is already being processed")]
    RequestAlreadyInProgress,

    #[error("No RPC request is currently in progress")]
    NoRequestInProgress,

    #[error("The incorrect RPC message type was received")]
    IncorrectMessageType,

    #[error("Incorrect channel ID (expected {expected}, got {actual})")]
    IncorrectChannel { expected: u8, actual: u8 },

    #[error("Incorrect sequence number (expected {expected}, got {actual})")]
    IncorrectSequenceNumber { expected: u16, actual: u16 },

    #[error("No acknowledgement was received")]
    NoAck,

    #[error("Timeout")]
    Timeout,

    #[error("Transport error")]
    TransportError,

    #[error("Serialization error")]
    SerializeError,

    #[error("Deserialization error")]
    DeserializeError,

    #[error("CRC mismatch")]
    CrcError,

    #[error("Frame is too long")]
    FrameTooLong,
}
