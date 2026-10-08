use spacepackets::{CcsdsPacketCreatorOwned, SpHeader};

use crate::chip::{Chip, Request};

pub fn create_tc(
    chip: Chip,
    request: Request,
    payload: &[u8],
) -> anyhow::Result<CcsdsPacketCreatorOwned> {
    let mut req_raw = chip.encode(request)?;
    req_raw.extend_from_slice(payload);
    let sp_header = SpHeader::new_from_apid(flashloader_types::APID);
    Ok(CcsdsPacketCreatorOwned::new_tc_with_checksum(
        sp_header, &req_raw,
    )?)
}

#[cfg(test)]
mod tests {
    use flashloader_types::{AppSel, va108xx, va416xx};
    use spacepackets::CcsdsPacketReader;

    use super::*;

    fn user_data(tc: CcsdsPacketCreatorOwned) -> Vec<u8> {
        let tc = tc.to_vec();
        CcsdsPacketReader::new_with_checksum(&tc)
            .unwrap()
            .user_data()
            .to_vec()
    }

    #[test]
    fn write_request_round_trip_va108xx() {
        let tc = create_tc(
            Chip::Va108xx,
            Request::WriteNvm { offset: 0x3000 },
            &[1, 2, 3],
        );
        let data = user_data(tc.unwrap());
        let (parsed, payload) = postcard::take_from_bytes::<va108xx::Request>(&data).unwrap();
        assert_eq!(parsed, va108xx::Request::WriteNvm { offset: 0x3000 });
        assert_eq!(payload, [1, 2, 3]);
    }

    #[test]
    fn write_request_round_trip_va416xx() {
        let tc = create_tc(
            Chip::Va416xx,
            Request::WriteNvm { offset: 0x4000 },
            &[1, 2, 3],
        );
        let data = user_data(tc.unwrap());
        let (parsed, payload) = postcard::take_from_bytes::<va416xx::Request>(&data).unwrap();
        assert_eq!(parsed, va416xx::Request::WriteNvm { offset: 0x4000 });
        assert_eq!(payload, [1, 2, 3]);
    }

    #[test]
    fn set_boot_slot_only_on_va108xx() {
        let data =
            user_data(create_tc(Chip::Va108xx, Request::SetBootSlot(AppSel::B), &[]).unwrap());
        let (parsed, _) = postcard::take_from_bytes::<va108xx::Request>(&data).unwrap();
        assert_eq!(parsed, va108xx::Request::SetBootSlot(AppSel::B));
        assert!(create_tc(Chip::Va416xx, Request::SetBootSlot(AppSel::B), &[]).is_err());
    }
}
