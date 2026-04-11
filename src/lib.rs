mod aggregator;
mod codec;
mod game_engine;
mod rng;
mod types;

use wasm_bindgen::prelude::*;

/// Run a batch of solitaire games and return aggregate metrics as a JSON string.
///
/// # Arguments
/// * `encoded`     - Binary wire format (see codec.ts for layout)
/// * `mech_keys`   - Mechanic group keys in order (used as metric key prefixes)
/// * `games`       - Number of games to simulate
/// * `max_turns`   - Max turns per game before ending (e.g. 30)
/// * `seed`        - Base RNG seed (each game gets seed + game_index)
///
/// # Returns
/// JSON string: `{ "avg_turns": 12.3, "land_in_play_t3": 2.8, ... }`
/// Returns an error JSON `{ "error": "..." }` on decode failure.
#[wasm_bindgen]
pub fn run_batch(
    encoded: &[u8],
    mech_keys: Vec<String>,
    games: u32,
    max_turns: u8,
    seed: u32,
) -> String {
    let (cards, mechanics) = match codec::decode(encoded) {
        Ok(v) => v,
        Err(e) => return format!("{{\"error\":\"{}\"}}", e),
    };

    let mut records = Vec::with_capacity(games as usize);

    for i in 0..games {
        let mut rng = rng::Rng::new(seed.wrapping_add(i));
        let rec = game_engine::run_game(&cards, &mechanics, &mut rng, max_turns);
        records.push(rec);
    }

    let metrics = aggregator::aggregate(&records, &mech_keys);

    // Serialise to JSON manually (no serde dependency)
    let mut json = String::from("{");
    let mut first = true;
    let mut keys: Vec<&String> = metrics.keys().collect();
    keys.sort(); // deterministic output order
    for k in keys {
        if !first { json.push(','); }
        first = false;
        let v = metrics[k];
        // Format: integers stay integer, floats to 4 decimal places
        if v.fract() == 0.0 && v.abs() < 1e9 {
            json.push_str(&format!("\"{}\":{}", k, v as i64));
        } else {
            json.push_str(&format!("\"{}\":{:.4}", k, v));
        }
    }
    json.push('}');
    json
}

/// Native (non-WASM) entry point — identical logic, no wasm_bindgen attribute.
/// Used by mamo-connector to run simulations locally without WASM overhead.
pub fn run_batch_native(
    encoded: &[u8],
    mech_keys: Vec<String>,
    games: u32,
    max_turns: u8,
    seed: u32,
) -> String {
    run_batch(encoded, mech_keys, games, max_turns, seed)
}

// ── Unit tests (run with `cargo test`, native target) ──────────────────────

#[cfg(test)]
mod tests {
    use crate::codec::decode;
    use crate::game_engine::run_game;
    use crate::rng::Rng;

    /// Build a minimal wire-format buffer with N identical cards and no mechanics.
    fn make_encoded(card_count: usize, flags: u8, cmc: u8, color_mask: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(card_count as u32).to_le_bytes()); // card_count
        buf.extend_from_slice(&0u32.to_le_bytes());                 // mechanic_count
        for _ in 0..card_count {
            buf.push(flags);       // byte 0: flags
            buf.push(cmc);         // byte 1: cmc
            buf.push(0);           // byte 2: power
            buf.push(0);           // byte 3: toughness
            buf.push(color_mask);  // byte 4: color_mask (mana produced)
            buf.push(0);           // byte 5: mana_w cost
            buf.push(0);           // byte 6: mana_u cost
            buf.push(0);           // byte 7: mana_b cost
            buf.push(0);           // byte 8: mana_r cost
            buf.push(0);           // byte 9: mana_g cost
            buf.push(cmc);         // byte 10: mana_generic cost = cmc (colorless spells)
            buf.push(0);           // byte 11: formation_role
            buf.extend_from_slice(&0u32.to_le_bytes()); // bytes 12-15: mechanic_mask
        }
        buf
    }

    #[test]
    fn test_decode_basic() {
        let buf = make_encoded(5, 0x01, 0, 0x01); // 5 lands
        let (cards, mechanics) = decode(&buf).unwrap();
        assert_eq!(cards.len(), 5);
        assert_eq!(mechanics.len(), 0);
        assert!(cards[0].is_land());
    }

    #[test]
    fn test_land_heavy_deck_mana_available() {
        // 37 lands + 63 generic spells (colorless 2-drops)
        let mut buf = Vec::new();
        let card_count = 100u32;
        buf.extend_from_slice(&card_count.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        for i in 0..100usize {
            let is_land = i < 37;
            let flags: u8 = if is_land { 0x09 } else { 0 }; // land=1 + mana_producing=8
            let cmc: u8   = if is_land { 0 } else { 2 };
            let color_mask: u8 = if is_land { 0x20 } else { 0 }; // colorless
            buf.push(flags); buf.push(cmc); buf.push(0); buf.push(0);
            buf.push(color_mask);
            buf.push(0); buf.push(0); buf.push(0); buf.push(0); buf.push(0);
            buf.push(if is_land { 0 } else { 2 }); // generic cost
            buf.push(0);
            buf.extend_from_slice(&0u32.to_le_bytes());
        }

        let (cards, mechanics) = decode(&buf).unwrap();
        assert_eq!(cards.len(), 100);

        let mut rng = Rng::new(42);
        let rec = run_game(&cards, &mechanics, &mut rng, 30);

        // After 6 turns we should have played several lands
        assert!(rec.lands_in_play[2] >= 2, "Should have ≥2 lands by turn 3, got {}", rec.lands_in_play[2]);
        assert!(rec.peak_mana >= 4, "Peak mana should be ≥4, got {}", rec.peak_mana);
        assert!(rec.total_turns > 0);
    }

    #[test]
    fn test_reproducibility() {
        let buf = make_encoded(30, 0x09, 0, 0x20); // 30 lands with colorless mana
        let (cards, mechanics) = decode(&buf).unwrap();

        let mut rng1 = Rng::new(12345);
        let mut rng2 = Rng::new(12345);
        let rec1 = run_game(&cards, &mechanics, &mut rng1, 20);
        let rec2 = run_game(&cards, &mechanics, &mut rng2, 20);

        assert_eq!(rec1.total_turns, rec2.total_turns);
        assert_eq!(rec1.lands_in_play, rec2.lands_in_play);
        assert_eq!(rec1.total_cards_drawn, rec2.total_cards_drawn);
    }

    #[test]
    fn test_aggregator_no_nan() {
        let buf = make_encoded(100, 0x09, 0, 0x20);
        let (cards, mechanics) = decode(&buf).unwrap();
        let mut records = Vec::new();
        for i in 0..50u32 {
            let mut rng = Rng::new(i);
            records.push(run_game(&cards, &mechanics, &mut rng, 20));
        }
        let metrics = crate::aggregator::aggregate(&records, &[]);
        for (k, v) in &metrics {
            assert!(!v.is_nan(), "Metric {} is NaN", k);
            assert!(!v.is_infinite(), "Metric {} is Infinite", k);
        }
        assert!(metrics.contains_key("avg_turns"));
        assert!(metrics.contains_key("land_in_play_t3"));
        assert!(metrics.contains_key("castable_options_t3"));
    }
}
