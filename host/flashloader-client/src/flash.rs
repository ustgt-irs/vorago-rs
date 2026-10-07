use std::path::Path;

use anyhow::Context;
use flashloader_types::AppSel;
use tmtc_utils::transport::serial::PacketTransportSerialCobs;

use crate::{
    Target,
    chip::{Chip, Request},
    elf::{check_total_size, log_segments_info, parse_elf},
    send_and_await_ok,
};

pub fn flash_image(
    transport: &mut PacketTransportSerialCobs,
    chip: Chip,
    target: Target,
    path: &Path,
) -> anyhow::Result<()> {
    let segments = parse_elf(chip, target, path)
        .with_context(|| format!("failed to parse ELF: {}", path.display()))?;

    let total_size: usize = segments.iter().map(|s| s.data.len()).sum();
    check_total_size(chip, target, total_size)?;
    log_segments_info(target, &segments, total_size, path);

    let chunk_size = chip.chunk_size();
    for seg in &segments {
        for (idx, chunk) in seg.data.chunks(chunk_size).enumerate() {
            let offset = seg.offset + (idx * chunk_size) as u32;
            log::info!(
                "Writing {} bytes @ {offset:#010x} ({}/{} of '{}')",
                chunk.len(),
                idx * chunk_size + chunk.len(),
                seg.data.len(),
                seg.name,
            );
            send_and_await_ok(transport, chip, Request::WriteNvm { offset }, chunk)?;
        }
    }

    let map = chip.memory_map();
    let app = match target {
        Target::Bl => {
            // The bootloader calculates and writes its own CRC on the first run.
            log::info!(
                "Blanking bootloader CRC @ {:#010x}",
                map.bootloader_crc_addr
            );
            send_and_await_ok(
                transport,
                chip,
                Request::WriteNvm {
                    offset: map.bootloader_crc_addr,
                },
                &vec![0; map.crc_len],
            )?;
            log::info!("Flash complete.");
            return Ok(());
        }
        Target::A => AppSel::A,
        Target::B => AppSel::B,
    };
    let slot = map.app(app);

    log::info!("Writing app size {total_size} @ {:#010x}", slot.size_addr);
    send_and_await_ok(
        transport,
        chip,
        Request::WriteNvm {
            offset: slot.size_addr,
        },
        &(total_size as u32).to_be_bytes(),
    )?;

    let crc = chip.image_crc(&segments);
    log::info!("Writing CRC {crc:02x?} @ {:#010x}", slot.crc_addr);
    send_and_await_ok(
        transport,
        chip,
        Request::WriteNvm {
            offset: slot.crc_addr,
        },
        &crc,
    )?;

    log::info!("Flash complete.");
    Ok(())
}
