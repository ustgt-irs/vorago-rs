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

    let (expected_base, max_size) = chip.memory_map().start_and_max_size(target);

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

        validate_segment_bounds(expected_base, max_size, addr, data.len())
            .map_err(|e| anyhow::anyhow!("segment {idx}: {e}"))?;

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

/// Rejects segment writes outside the target's usable image region.
/// The bounds exclude reserved image metadata.
fn validate_segment_bounds(
    partition_start: u32,
    partition_size: u32,
    segment_start: u32,
    segment_len: usize,
) -> anyhow::Result<()> {
    let partition_end = partition_start
        .checked_add(partition_size)
        .ok_or_else(|| anyhow::anyhow!("partition address overflow"))?;

    let segment_len =
        u32::try_from(segment_len).map_err(|_| anyhow::anyhow!("segment length exceeds u32"))?;

    let segment_end = segment_start
        .checked_add(segment_len)
        .ok_or_else(|| anyhow::anyhow!("segment address overflow"))?;

    if segment_start < partition_start || segment_end > partition_end {
        anyhow::bail!(
            "segment range {segment_start:#010x}..{segment_end:#010x}              outside partition {partition_start:#010x}..{partition_end:#010x}"
        );
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_elf(second_addr: u32) -> Vec<u8> {
        let mut elf = vec![0u8; 0x108];

        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 1; // ELF32
        elf[5] = 1; // little endian
        elf[6] = 1; // ELF version

        fn put16(buf: &mut [u8], off: usize, value: u16) {
            buf[off..off + 2].copy_from_slice(&value.to_le_bytes());
        }

        fn put32(buf: &mut [u8], off: usize, value: u32) {
            buf[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }

        put16(&mut elf, 16, 2); // ET_EXEC
        put16(&mut elf, 18, 40); // EM_ARM
        put32(&mut elf, 20, 1);
        put32(&mut elf, 28, 52); // program header offset
        put16(&mut elf, 40, 52); // ELF header size
        put16(&mut elf, 42, 32); // program header entry size
        put16(&mut elf, 44, 2); // two program headers
        put16(&mut elf, 46, 40); // section header entry size

        for (idx, addr) in [0x4000u32, second_addr].iter().enumerate() {
            let ph = 52 + idx * 32;
            put32(&mut elf, ph, 1); // PT_LOAD
            put32(&mut elf, ph + 4, 0x100 + (idx as u32) * 4);
            put32(&mut elf, ph + 8, *addr);
            put32(&mut elf, ph + 12, *addr);
            put32(&mut elf, ph + 16, 4);
            put32(&mut elf, ph + 20, 4);
            put32(&mut elf, ph + 24, 5);
            put32(&mut elf, ph + 28, 4);
        }

        elf[0x100..0x104].copy_from_slice(&[1, 2, 3, 4]);
        elf[0x104..0x108].copy_from_slice(&[5, 6, 7, 8]);
        elf
    }

    fn parse_test_elf(bytes: &[u8]) -> anyhow::Result<Vec<LoadableSegment>> {
        let path = std::env::temp_dir().join(format!(
            "vorago-elf-bounds-test-{}-{:?}.elf",
            std::process::id(),
            std::thread::current().id()
        ));

        std::fs::write(&path, bytes)?;
        let result = parse_elf(Chip::Va416xx, Target::A, &path);
        std::fs::remove_file(&path)?;
        result
    }

    #[test]
    fn parser_accepts_two_in_partition_segments() {
        let elf = make_test_elf(0x4004);
        let result = parse_test_elf(&elf).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn parser_rejects_second_segment_in_slot_b() {
        let elf = make_test_elf(0x22000);
        let err = match parse_test_elf(&elf) {
            Ok(_) => panic!("out-of-partition ELF was accepted"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("outside partition"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn valid_segment_within_va416xx_slot_a() {
        let (start, size) = Chip::Va416xx.memory_map().start_and_max_size(Target::A);

        assert!(validate_segment_bounds(start, size, start, 256).is_ok());
    }

    #[test]
    fn segment_ending_at_partition_boundary_is_valid() {
        let (start, size) = Chip::Va416xx.memory_map().start_and_max_size(Target::A);

        assert!(validate_segment_bounds(start, size, start + size - 4, 4).is_ok());
    }

    #[test]
    fn segment_crossing_metadata_is_rejected() {
        let (start, size) = Chip::Va416xx.memory_map().start_and_max_size(Target::A);

        assert!(validate_segment_bounds(start, size, start + size - 4, 8).is_err());
    }

    #[test]
    fn segment_before_partition_is_rejected() {
        let (start, size) = Chip::Va416xx.memory_map().start_and_max_size(Target::A);

        assert!(validate_segment_bounds(start, size, start - 4, 4).is_err());
    }

    #[test]
    fn segment_in_other_slot_is_rejected() {
        let (start, size) = Chip::Va416xx.memory_map().start_and_max_size(Target::A);

        let other_slot =
            Chip::Va416xx.memory_map().app_a.start + Chip::Va416xx.memory_map().app_a.max_size + 8;

        assert!(validate_segment_bounds(start, size, other_slot, 16).is_err());
    }

    #[test]
    fn segment_address_overflow_is_rejected() {
        assert!(validate_segment_bounds(0, u32::MAX, u32::MAX - 2, 8).is_err());
    }

    #[test]
    fn va108xx_partition_boundary_is_enforced() {
        let (start, size) = Chip::Va108xx.memory_map().start_and_max_size(Target::B);

        assert!(validate_segment_bounds(start, size, start, 32).is_ok());
        assert!(validate_segment_bounds(start, size, start + size - 1, 2).is_err());
    }
}
