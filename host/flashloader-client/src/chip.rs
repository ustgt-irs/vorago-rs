//! Everything which differs between the VA108xx and the VA416xx flashloader.
use anyhow::bail;
use crc::{CRC_16_IBM_3740, CRC_32_ISO_HDLC, Crc};
use flashloader_types::{AppSel, va108xx, va416xx};

use crate::{Target, elf::LoadableSegment};

#[derive(clap::ValueEnum, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Chip {
    Va108xx,
    Va416xx,
}

/// A flashloader request, independent of the chip.
#[derive(Clone, Copy, Debug)]
pub enum Request {
    Ping,
    Corrupt(AppSel),
    /// The data to write follows the request in the packet.
    WriteNvm {
        offset: u32,
    },
    SetBootSlot(AppSel),
}

pub struct AppSlot {
    pub start: u32,
    pub max_size: u32,
    pub size_addr: u32,
    pub crc_addr: u32,
}

pub struct MemoryMap {
    pub bootloader_start: u32,
    pub bootloader_max_size: u32,
    pub bootloader_crc_addr: u32,
    /// Width of the bootloader and image CRCs in bytes.
    pub crc_len: usize,
    pub app_a: AppSlot,
    pub app_b: AppSlot,
}

impl MemoryMap {
    pub fn start_and_max_size(&self, target: Target) -> (u32, u32) {
        match target {
            Target::Bl => (self.bootloader_start, self.bootloader_max_size),
            Target::A => (self.app_a.start, self.app_a.max_size),
            Target::B => (self.app_b.start, self.app_b.max_size),
        }
    }

    pub fn app(&self, app: AppSel) -> &AppSlot {
        match app {
            AppSel::A => &self.app_a,
            AppSel::B => &self.app_b,
        }
    }
}

const VA108XX_CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_3740);
const VA416XX_CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

const VA108XX_MAP: MemoryMap = MemoryMap {
    bootloader_start: va108xx::BOOTLOADER_START_ADDR,
    bootloader_max_size: va108xx::BOOTLOADER_MAX_SIZE,
    bootloader_crc_addr: va108xx::BOOTLOADER_CRC_ADDR,
    crc_len: 2,
    app_a: AppSlot {
        start: va108xx::APP_A_START_ADDR,
        max_size: va108xx::APP_A_MAX_SIZE,
        size_addr: va108xx::APP_A_SIZE_ADDR,
        crc_addr: va108xx::APP_A_CRC_ADDR,
    },
    app_b: AppSlot {
        start: va108xx::APP_B_START_ADDR,
        max_size: va108xx::APP_B_MAX_SIZE,
        size_addr: va108xx::APP_B_SIZE_ADDR,
        crc_addr: va108xx::APP_B_CRC_ADDR,
    },
};

const VA416XX_MAP: MemoryMap = MemoryMap {
    bootloader_start: va416xx::BOOTLOADER_START_ADDR,
    bootloader_max_size: va416xx::BOOTLOADER_MAX_SIZE,
    bootloader_crc_addr: va416xx::BOOTLOADER_CRC_ADDR,
    crc_len: 4,
    app_a: AppSlot {
        start: va416xx::APP_A_START_ADDR,
        max_size: va416xx::APP_A_MAX_SIZE,
        size_addr: va416xx::APP_A_SIZE_ADDR,
        crc_addr: va416xx::APP_A_CRC_ADDR,
    },
    app_b: AppSlot {
        start: va416xx::APP_B_START_ADDR,
        max_size: va416xx::APP_B_MAX_SIZE,
        size_addr: va416xx::APP_B_SIZE_ADDR,
        crc_addr: va416xx::APP_B_CRC_ADDR,
    },
};

impl Chip {
    pub fn memory_map(self) -> &'static MemoryMap {
        match self {
            Chip::Va108xx => &VA108XX_MAP,
            Chip::Va416xx => &VA416XX_MAP,
        }
    }

    /// Keeps a write TC below the TC size limit of the flashloader.
    pub fn chunk_size(self) -> usize {
        match self {
            // The limit is 524 bytes.
            Chip::Va108xx => 256,
            // The limit is 1024 bytes.
            Chip::Va416xx => 896,
        }
    }

    /// CRC over all segments, in the byte order the bootloader expects.
    pub fn image_crc(self, segments: &[LoadableSegment]) -> Vec<u8> {
        match self {
            Chip::Va108xx => {
                let mut digest = VA108XX_CRC.digest();
                for seg in segments {
                    digest.update(&seg.data);
                }
                digest.finalize().to_be_bytes().to_vec()
            }
            Chip::Va416xx => {
                let mut digest = VA416XX_CRC.digest();
                for seg in segments {
                    digest.update(&seg.data);
                }
                digest.finalize().to_be_bytes().to_vec()
            }
        }
    }

    /// Serializes the request with the request type of the chip's flashloader.
    pub fn encode(self, request: Request) -> anyhow::Result<Vec<u8>> {
        let raw = match self {
            Chip::Va108xx => postcard::to_allocvec(&match request {
                Request::Ping => va108xx::Request::Ping,
                Request::Corrupt(app) => va108xx::Request::Corrupt(app),
                Request::WriteNvm { offset } => va108xx::Request::WriteNvm { offset },
                Request::SetBootSlot(app) => va108xx::Request::SetBootSlot(app),
            }),
            Chip::Va416xx => postcard::to_allocvec(&match request {
                Request::Ping => va416xx::Request::Ping,
                Request::Corrupt(app) => va416xx::Request::Corrupt(app),
                Request::WriteNvm { offset } => va416xx::Request::WriteNvm { offset },
                Request::SetBootSlot(_) => {
                    bail!("the VA416xx flashloader does not support setting the boot slot")
                }
            }),
        }?;
        Ok(raw)
    }
}
