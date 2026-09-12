mod aggregator;
/// `pub` so `mamo-Connector` (a Cargo path dependency on this crate, not just the published
/// WASM package) can decode a buffer it built itself in tests — a direct cross-crate check
/// that its own `encode_deck_input` (commands.rs) agrees byte-for-byte with this decoder,
/// rather than each side trusting its own understanding of the wire format. Not part of the
/// WASM-exported surface: wasm-pack/wasm_bindgen only binds `#[wasm_bindgen]` items to JS, so
/// this has no effect on the published `@killriam/mamo-sim` npm package's API.
pub mod codec;
mod game_engine;
mod rng;
pub mod types;

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
    let (cards, mechanics, mulligan_config) = match codec::decode(encoded) {
        Ok(v) => v,
        Err(e) => return format!("{{\"error\":\"{}\"}}", e),
    };

    let mut records = Vec::with_capacity(games as usize);

    for i in 0..games {
        let mut rng = rng::Rng::new(seed.wrapping_add(i));
        let rec = game_engine::run_game(&cards, &mechanics, &mulligan_config, &mut rng, max_turns);
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

    /// Appends the wire format's mulligan-config header (default curve values, no explicit
    /// thresholds — callers fall back to `MulliganConfig::default_config()`).
    fn push_default_mulligan_header(buf: &mut Vec<u8>) {
        push_mulligan_header(buf, 1.0, &[0.85, 0.8, 0.75, 0.6, 0.45, 0.4, 0.35, 0.3], &[]);
    }

    /// Appends a fully custom wire-format mulligan-config header. `mv_values` must have
    /// exactly 8 entries (mana value 0-6, then a 7+ catch-all), matching codec.rs's decoder.
    fn push_mulligan_header(buf: &mut Vec<u8>, land: f32, mv_values: &[f32; 8], thresholds: &[(u8, f32)]) {
        buf.extend_from_slice(&land.to_le_bytes());
        for v in mv_values {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.push(thresholds.len() as u8);
        buf.extend_from_slice(&[0u8; 3]); // reserved padding
        for (round, min_value) in thresholds {
            buf.push(*round);
            buf.extend_from_slice(&min_value.to_le_bytes());
        }
    }

    /// Build a minimal wire-format buffer with N identical cards and no mechanics.
    fn make_encoded(card_count: usize, flags: u8, cmc: u8, color_mask: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(card_count as u32).to_le_bytes()); // card_count
        buf.extend_from_slice(&0u32.to_le_bytes());                 // mechanic_count
        push_default_mulligan_header(&mut buf);
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
        let (cards, mechanics, mulligan) = decode(&buf).unwrap();
        assert_eq!(cards.len(), 5);
        assert_eq!(mechanics.len(), 0);
        assert!(cards[0].is_land());
        assert_eq!(mulligan.land_value, 1.0);
    }

    #[test]
    fn test_land_heavy_deck_mana_available() {
        // 37 lands + 63 generic spells (colorless 2-drops)
        let mut buf = Vec::new();
        let card_count = 100u32;
        buf.extend_from_slice(&card_count.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        push_default_mulligan_header(&mut buf);
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

        let (cards, mechanics, mulligan) = decode(&buf).unwrap();
        assert_eq!(cards.len(), 100);

        let mut rng = Rng::new(42);
        let rec = run_game(&cards, &mechanics, &mulligan, &mut rng, 30);

        // After 6 turns we should have played several lands
        assert!(rec.lands_in_play[2] >= 2, "Should have ≥2 lands by turn 3, got {}", rec.lands_in_play[2]);
        assert!(rec.peak_mana >= 4, "Peak mana should be ≥4, got {}", rec.peak_mana);
        assert!(rec.total_turns > 0);
    }

    #[test]
    fn test_reproducibility() {
        let buf = make_encoded(30, 0x09, 0, 0x20); // 30 lands with colorless mana
        let (cards, mechanics, mulligan) = decode(&buf).unwrap();

        let mut rng1 = Rng::new(12345);
        let mut rng2 = Rng::new(12345);
        let rec1 = run_game(&cards, &mechanics, &mulligan, &mut rng1, 20);
        let rec2 = run_game(&cards, &mechanics, &mulligan, &mut rng2, 20);

        assert_eq!(rec1.total_turns, rec2.total_turns);
        assert_eq!(rec1.lands_in_play, rec2.lands_in_play);
        assert_eq!(rec1.total_cards_drawn, rec2.total_cards_drawn);
    }

    #[test]
    fn test_aggregator_no_nan() {
        let buf = make_encoded(100, 0x09, 0, 0x20);
        let (cards, mechanics, mulligan) = decode(&buf).unwrap();
        let mut records = Vec::new();
        for i in 0..50u32 {
            let mut rng = Rng::new(i);
            records.push(run_game(&cards, &mechanics, &mulligan, &mut rng, 20));
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

    #[test]
    fn test_mulligan_config_default_fallback() {
        let default = crate::types::MulliganConfig::default_config();
        assert_eq!(default.min_value_for_round(0), 3.5);
        assert_eq!(default.min_value_for_round(1), 3.0);

        // A config that only configures round 0 falls back to the built-in default for
        // any other round — mirrors Forge's DecklistMulliganEvaluator fallback behavior.
        let mut partial = default;
        partial.threshold_count = 1;
        partial.thresholds[0] = (0, 10.0);
        assert_eq!(partial.min_value_for_round(0), 10.0);
        assert_eq!(partial.min_value_for_round(1), 3.0);
    }

    #[test]
    fn test_mulligan_decision_driven_by_configured_thresholds() {
        // A 40-card deck of nothing but 5-CMC non-lands (mv5 tier, no lands at all) —
        // deliberately unkeepable under any land-aware heuristic.
        let mut buf = Vec::new();
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        push_mulligan_header(
            &mut buf,
            1.0,
            &[0.85, 0.8, 0.75, 0.6, 0.45, /* mv5 */ 0.3, 0.35, 0.3],
            &[(0, 3.5), (1, 3.0)],
        );
        for _ in 0..40u32 {
            buf.push(0);       // flags: not a land
            buf.push(5);       // cmc
            buf.push(0); buf.push(0); buf.push(0);
            buf.extend_from_slice(&[0u8; 6]);
            buf.push(0);
            buf.extend_from_slice(&0u32.to_le_bytes());
        }
        let (cards, mechanics, default_mulligan) = decode(&buf).unwrap();

        let mut rng = Rng::new(7);
        let rec_default = run_game(&cards, &mechanics, &default_mulligan, &mut rng, 5);
        // 7 mv5 cards score 7*0.3=2.1, under the 3.5 round-0 threshold — must mulligan.
        assert!(rec_default.took_mulligan(), "default config should mulligan an all-5-drop hand");

        let mut lenient_buf = Vec::new();
        lenient_buf.extend_from_slice(&40u32.to_le_bytes());
        lenient_buf.extend_from_slice(&0u32.to_le_bytes());
        push_mulligan_header(
            &mut lenient_buf,
            1.0,
            &[0.85, 0.8, 0.75, 0.6, 0.45, /* mv5 */ 5.0, 0.35, 0.3],
            &[(0, 1.0), (1, 1.0)],
        );
        for _ in 0..40u32 {
            lenient_buf.push(0);
            lenient_buf.push(5);
            lenient_buf.push(0); lenient_buf.push(0); lenient_buf.push(0);
            lenient_buf.extend_from_slice(&[0u8; 6]);
            lenient_buf.push(0);
            lenient_buf.extend_from_slice(&0u32.to_le_bytes());
        }
        let (cards2, mechanics2, lenient_mulligan) = decode(&lenient_buf).unwrap();
        let mut rng2 = Rng::new(7);
        let rec_lenient = run_game(&cards2, &mechanics2, &lenient_mulligan, &mut rng2, 5);
        // Same deck, same seed — only the deck's configured mulligan values changed
        // (mv5 value 0.3→5.0, threshold 3.5→1.0): 7*5.0=35 >= 1.0, hand is kept.
        assert!(!rec_lenient.took_mulligan(), "lenient config should keep the same hand");
    }
}
