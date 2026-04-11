use crate::types::{SimCard, SimMechanic};

/// Decodes the binary wire format produced by codec.ts into card and mechanic slices.
///
/// Wire layout:
///   [4 bytes] card_count  (u32 LE)
///   [4 bytes] mechanic_count (u32 LE)
///   [card_count × 16 bytes] card records
///   [mechanic_count × 12 bytes] mechanic records
pub fn decode(data: &[u8]) -> Result<(Vec<SimCard>, Vec<SimMechanic>), &'static str> {
    if data.len() < 8 {
        return Err("Input too short — missing header");
    }

    let card_count    = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mechanic_count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    let card_bytes_needed     = card_count * 16;
    let mechanic_bytes_needed = mechanic_count * 12;
    let total_needed = 8 + card_bytes_needed + mechanic_bytes_needed;

    if data.len() < total_needed {
        return Err("Input too short — truncated card or mechanic data");
    }

    // ── Decode cards ─────────────────────────────────────────────────
    let mut cards = Vec::with_capacity(card_count);
    let card_base = 8usize;

    for i in 0..card_count {
        let o = card_base + i * 16;
        let mechanic_mask = u32::from_le_bytes([data[o+12], data[o+13], data[o+14], data[o+15]]);
        cards.push(SimCard {
            flags:         data[o],
            cmc:           data[o+1],
            power:         data[o+2],
            toughness:     data[o+3],
            color_mask:    data[o+4],
            mana_w:        data[o+5],
            mana_u:        data[o+6],
            mana_b:        data[o+7],
            mana_r:        data[o+8],
            mana_g:        data[o+9],
            mana_generic:  data[o+10],
            formation_role: data[o+11],
            mechanic_mask,
        });
    }

    // ── Decode mechanics ─────────────────────────────────────────────
    let mut mechanics = Vec::with_capacity(mechanic_count);
    let mech_base = card_base + card_bytes_needed;

    for i in 0..mechanic_count {
        let o = mech_base + i * 12;
        let card_mask   = u32::from_le_bytes([data[o+4], data[o+5], data[o+6], data[o+7]]);
        let prereq_mask = u32::from_le_bytes([data[o+8], data[o+9], data[o+10], data[o+11]]);
        mechanics.push(SimMechanic {
            activation:  data[o],
            advantage:   data[o+1],
            dimension:   data[o+2],
            // data[o+3] reserved
            card_mask,
            prereq_mask,
        });
    }

    Ok((cards, mechanics))
}
