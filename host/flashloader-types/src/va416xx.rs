use crate::AppSel;

pub const BOOTLOADER_START_ADDR: u32 = 0x0000_0000;
pub const BOOTLOADER_END_ADDR: u32 = 0x0000_4000;
pub const BOOTLOADER_CRC_ADDR: u32 = BOOTLOADER_END_ADDR - 4;
pub const BOOTLOADER_MAX_SIZE: u32 = BOOTLOADER_END_ADDR - BOOTLOADER_START_ADDR - 4;

pub const APP_A_START_ADDR: u32 = BOOTLOADER_END_ADDR;
pub const APP_B_END_ADDR: u32 = 0x0004_0000;
pub const IMG_SLOT_SIZE: u32 = (APP_B_END_ADDR - APP_A_START_ADDR) / 2;

pub const APP_A_END_ADDR: u32 = APP_A_START_ADDR + IMG_SLOT_SIZE;
pub const APP_A_SIZE_ADDR: u32 = APP_A_END_ADDR - 8;
pub const APP_A_CRC_ADDR: u32 = APP_A_END_ADDR - 4;
pub const APP_A_MAX_SIZE: u32 = IMG_SLOT_SIZE - 8;

pub const APP_B_START_ADDR: u32 = APP_A_END_ADDR;
pub const APP_B_SIZE_ADDR: u32 = APP_B_END_ADDR - 8;
pub const APP_B_CRC_ADDR: u32 = APP_B_END_ADDR - 4;
pub const APP_B_MAX_SIZE: u32 = IMG_SLOT_SIZE - 8;

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    Ping,
    Corrupt(AppSel),
    /// The data to write follows the serialized request in the packet.
    WriteNvm {
        offset: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_map() {
        assert_eq!(APP_A_END_ADDR, 0x22000);
        assert_eq!(APP_A_SIZE_ADDR, 0x21FF8);
        assert_eq!(APP_A_CRC_ADDR, 0x21FFC);
        assert_eq!(APP_B_SIZE_ADDR, 0x3FFF8);
        assert_eq!(APP_B_CRC_ADDR, 0x3FFFC);
    }
}
