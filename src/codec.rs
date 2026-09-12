use crate::types::{MulliganConfig, SimCard, SimMechanic, MAX_MULLIGAN_THRESHOLDS};

/// Decodes the binary wire format produced by codec.ts into card/mechanic slices plus the
/// deck's mulligan config.
///
/// Wire layout:
///   [4 bytes] card_count  (u32 LE)
///   [4 bytes] mechanic_count (u32 LE)
///   [4 bytes] mulligan land_value (f32 LE)
///   [4 bytes] mulligan mv0_value (f32 LE)
///   [4 bytes] mulligan mv1_value (f32 LE)
///   [4 bytes] mulligan mv2_value (f32 LE)
///   [4 bytes] mulligan mv3_value (f32 LE)
///   [4 bytes] mulligan mv4_value (f32 LE)
///   [4 bytes] mulligan mv5_value (f32 LE)
///   [4 bytes] mulligan mv6_value (f32 LE)
///   [4 bytes] mulligan mv7Plus_value (f32 LE)
///   [1 byte]  mulligan threshold_count (u8)
///   [3 bytes] reserved padding
///   [threshold_count × 5 bytes] mulligan threshold records: (round: u8, min_value: f32 LE)
///   [card_count × 16 bytes] card records
///   [mechanic_count × 12 bytes] mechanic records
pub fn decode(data: &[u8]) -> Result<(Vec<SimCard>, Vec<SimMechanic>, MulliganConfig), &'static str> {
    if data.len() < 48 {
        return Err("Input too short — missing header");
    }

    let card_count    = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mechanic_count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    let land_value = f32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let mut mv_values = [0f32; 8];
    for (i, v) in mv_values.iter_mut().enumerate() {
        let o = 12 + i * 4;
        *v = f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    }
    let threshold_count_wire = data[44] as usize;
    // data[45..48] reserved

    let thresholds_base = 48usize;
    let thresholds_bytes_needed = threshold_count_wire * 5;
    if data.len() < thresholds_base + thresholds_bytes_needed {
        return Err("Input too short — truncated mulligan thresholds");
    }

    let mut thresholds = [(0u8, 0f32); MAX_MULLIGAN_THRESHOLDS];
    for i in 0..threshold_count_wire {
        let o = thresholds_base + i * 5;
        let round = data[o];
        let min_value = f32::from_le_bytes([data[o + 1], data[o + 2], data[o + 3], data[o + 4]]);
        if i < MAX_MULLIGAN_THRESHOLDS {
            thresholds[i] = (round, min_value);
        }
    }
    let mulligan = MulliganConfig {
        land_value,
        mv_values,
        thresholds,
        threshold_count: threshold_count_wire.min(MAX_MULLIGAN_THRESHOLDS) as u8,
    };

    let card_base = thresholds_base + thresholds_bytes_needed;
    let card_bytes_needed     = card_count * 16;
    let mechanic_bytes_needed = mechanic_count * 12;
    let total_needed = card_base + card_bytes_needed + mechanic_bytes_needed;

    if data.len() < total_needed {
        return Err("Input too short — truncated card or mechanic data");
    }

    // ── Decode cards ─────────────────────────────────────────────────
    let mut cards = Vec::with_capacity(card_count);

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

    Ok((cards, mechanics, mulligan))
}
