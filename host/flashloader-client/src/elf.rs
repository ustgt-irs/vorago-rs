use std::path::Path;

use crate::{Target, chip::Chip};

pub struct LoadableSegment {
    pub name: String,
    pub offset: u32,
    pub data: Vec<u8>,
}

/// Collects the loadable segments of the ELF file at their load addresses.
///
/// The load address can differ from the run address. For example, the initial values of `.data`
/// are stored in the image and copied to RAM at startup.
pub fn parse_elf(chip: Chip, target: Target, path: &Path) -> anyhow::Result<Vec<LoadableSegment>> {
    use object::read::elf::{ElfFile32, ProgramHeader};
    use object::{Endianness, Object, ObjectSection, elf::PT_LOAD};

    let raw = std::fs::read(path)?;
    let elf = ElfFile32::<Endianness>::parse(raw.as_slice())?;
    let endian = elf.endian();

    let (expected_base, _) = chip.memory_map().start_and_max_size(target);

    let mut segments = Vec::new();

    for (idx, phdr) in elf.elf_program_headers().iter().enumerate() {
        if phdr.p_type(endian) != PT_LOAD {
            continue;
        }
        let data = phdr
            .data(endian, raw.as_slice())
            .map_err(|()| anyhow::anyhow!("invalid data range in segment {idx}"))?;
        if data.is_empty() {
            continue;
        }

        let addr = phdr.p_paddr(endian);

        if segments.is_empty() && addr != expected_base {
            anyhow::bail!(
                "unexpected base address {addr:#010x} for {target:?}, expected {expected_base:#010x}"
            );
        }

        // Matched by file offset, because the section addresses are run addresses.
        let file_start = u64::from(phdr.p_offset(endian));
        let file_end = file_start + data.len() as u64;
        let name = elf
            .sections()
            .find(|s| {
                s.file_range()
                    .is_some_and(|(offset, _)| offset >= file_start && offset < file_end)
                    && !s.name().unwrap_or("").is_empty()
            })
            .and_then(|s| s.name().ok().map(str::to_owned))
            .unwrap_or_else(|| format!("<segment {idx}>"));

        segments.push(LoadableSegment {
            name,
            offset: addr,
            data: data.to_vec(),
        });
    }

    Ok(segments)
}

pub fn check_total_size(chip: Chip, target: Target, total_size: usize) -> anyhow::Result<()> {
    let (_, max) = chip.memory_map().start_and_max_size(target);
    let max = max as usize;

    if total_size > max {
        anyhow::bail!("{target:?} image is {total_size} bytes, exceeds maximum of {max} bytes");
    }
    Ok(())
}

pub fn log_segments_info(
    target: Target,
    segments: &[LoadableSegment],
    total_size: usize,
    path: &Path,
) {
    log::info!(
        "Flashing {target:?} with image '{}', {} segment(s), {total_size} bytes total",
        path.display(),
        segments.len()
    );
    for (i, seg) in segments.iter().enumerate() {
        log::info!(
            "  segment {i}: '{}' @ {:#010x}, {} bytes",
            seg.name,
            seg.offset,
            seg.data.len()
        );
    }
}
