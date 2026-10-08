//! Packet types and memory maps shared by the VA108xx and VA416xx flashloaders and their client.
#![no_std]

use arbitrary_int::u11;
use cobs::DestBufTooSmallError;
use spacepackets::{
    CcsdsPacketCreationError, CcsdsPacketCreatorWithReservedData, SpacePacketHeader,
};

pub mod va108xx;
pub mod va416xx;

pub const APID: u11 = u11::new(0x01);

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AppSel {
    A,
    B,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    Ok,
}

pub fn create_tm_packet(
    buf: &mut [u8],
    sp_header: SpacePacketHeader,
    response: Response,
) -> Result<usize, CcsdsPacketCreationError> {
    let packet_data_size = postcard::experimental::serialized_size(&response).unwrap();
    let mut creator =
        CcsdsPacketCreatorWithReservedData::new_tm_with_checksum(sp_header, packet_data_size, buf)?;
    postcard::to_slice(&response, creator.packet_data_mut()).unwrap();
    Ok(creator.finish())
}

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PacketCreationAndEncodingError {
    #[error("packet creation failed: {0}")]
    Creation(#[from] CcsdsPacketCreationError),
    #[error("destination buffer too small: {0}")]
    Encoding(#[from] DestBufTooSmallError),
}

pub fn create_encoded_tm_packet(
    buf: &mut [u8],
    encoded_buf: &mut [u8],
    sp_header: SpacePacketHeader,
    response: Response,
) -> Result<usize, PacketCreationAndEncodingError> {
    let packet_len = create_tm_packet(buf, sp_header, response)?;
    let encoded_len = cobs::try_encode_including_sentinels(&buf[0..packet_len], encoded_buf)?;
    Ok(encoded_len)
}
