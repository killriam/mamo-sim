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
    // Computed once per batch, not per game — every game in a batch shares the same deck, so
    // recomputing this per game would redo identical work `games` times over.
    let pip_weights = types::compute_deck_pip_weights(&cards);

    let mut records = Vec::with_capacity(games as usize);

    for i in 0..games {
        let mut rng = rng::Rng::new(seed.wrapping_add(i));
        let rec = game_engine::run_game(
            &cards,
            &mechanics,
            &mulligan_config,
            &pip_weights,
            &mut rng,
            max_turns,
        );
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

    /// Appends the wire format's mulligan-config header (default curve values, default Mana
    /// Base band, no explicit thresholds — callers fall back to `MulliganConfig::default_config()`).
    fn push_default_mulligan_header(buf: &mut Vec<u8>) {
        push_mulligan_header_full(
            buf,
            1.0,
            &[0.85, 0.8, 0.75, 0.6, 0.45, 0.4, 0.35, 0.3],
            3.0,
            4.0,
            &[],
        );
    }

    /// Appends a fully custom wire-format mulligan-config header. `mv_values` must have
    /// exactly 8 entries (mana value 0-6, then a 7+ catch-all), matching codec.rs's decoder.
    fn push_mulligan_header_full(
        buf: &mut Vec<u8>,
        land: f32,
        mv_values: &[f32; 8],
        mana_base_min: f32,
        mana_base_max: f32,
        thresholds: &[(u8, f32)],
    ) {
        buf.extend_from_slice(&land.to_le_bytes());
        for v in mv_values {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&mana_base_min.to_le_bytes());
        buf.extend_from_slice(&mana_base_max.to_le_bytes());
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
        let pip_weights = crate::types::compute_deck_pip_weights(&cards);

        let mut rng = Rng::new(42);
        let rec = run_game(&cards, &mechanics, &mulligan, &pip_weights, &mut rng, 30);

        // After 6 turns we should have played several lands
        assert!(rec.lands_in_play[2] >= 2, "Should have ≥2 lands by turn 3, got {}", rec.lands_in_play[2]);
        assert!(rec.peak_mana >= 4, "Peak mana should be ≥4, got {}", rec.peak_mana);
        assert!(rec.total_turns > 0);
    }

    #[test]
    fn test_reproducibility() {
        let buf = make_encoded(30, 0x09, 0, 0x20); // 30 lands with colorless mana
        let (cards, mechanics, mulligan) = decode(&buf).unwrap();
        let pip_weights = crate::types::compute_deck_pip_weights(&cards);

        let mut rng1 = Rng::new(12345);
        let mut rng2 = Rng::new(12345);
        let rec1 = run_game(&cards, &mechanics, &mulligan, &pip_weights, &mut rng1, 20);
        let rec2 = run_game(&cards, &mechanics, &mulligan, &pip_weights, &mut rng2, 20);

        assert_eq!(rec1.total_turns, rec2.total_turns);
        assert_eq!(rec1.lands_in_play, rec2.lands_in_play);
        assert_eq!(rec1.total_cards_drawn, rec2.total_cards_drawn);
    }

    #[test]
    fn test_aggregator_no_nan() {
        let buf = make_encoded(100, 0x09, 0, 0x20);
        let (cards, mechanics, mulligan) = decode(&buf).unwrap();
        let pip_weights = crate::types::compute_deck_pip_weights(&cards);
        let mut records = Vec::new();
        for i in 0..50u32 {
            let mut rng = Rng::new(i);
            records.push(run_game(&cards, &mechanics, &mulligan, &pip_weights, &mut rng, 20));
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
    fn test_mulligan_decision_driven_by_mana_base_band() {
        // A 40-card deck of nothing but basic lands — every possible 7-card hand has Mana Base
        // exactly 7.0, deterministically (all 40 cards are identical, so shuffle order can't
        // change the outcome).
        let mut buf = Vec::new();
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        push_mulligan_header_full(
            &mut buf,
            1.0,
            &[0.85, 0.8, 0.75, 0.6, 0.45, 0.4, 0.35, 0.3],
            3.0,
            4.0, // default band — a 7-land hand (Mana Base 7.0) is well over the max
            &[],
        );
        for _ in 0..40u32 {
            buf.push(0x41); // flags: isLand (0x01) + isBasicLand (0x40)
            buf.push(0);    // cmc
            buf.push(0); buf.push(0); buf.push(0); // toughness, color_mask (unused: basic-land branch short-circuits)
            buf.extend_from_slice(&[0u8; 6]);
            buf.push(0);
            buf.extend_from_slice(&0u32.to_le_bytes());
        }
        let (cards, mechanics, default_mulligan) = decode(&buf).unwrap();
        let pip_weights = crate::types::compute_deck_pip_weights(&cards);

        let mut rng = Rng::new(7);
        let rec_default = run_game(&cards, &mechanics, &default_mulligan, &pip_weights, &mut rng, 5);
        assert!(
            rec_default.took_mulligan(),
            "an all-land hand (Mana Base 7.0) should be rejected as too much mana under the default 3.0-4.0 band"
        );

        let mut wide_buf = Vec::new();
        wide_buf.extend_from_slice(&40u32.to_le_bytes());
        wide_buf.extend_from_slice(&0u32.to_le_bytes());
        push_mulligan_header_full(
            &mut wide_buf,
            1.0,
            &[0.85, 0.8, 0.75, 0.6, 0.45, 0.4, 0.35, 0.3],
            3.0,
            10.0, // widened band — a 7-land hand now fits comfortably
            &[],
        );
        for _ in 0..40u32 {
            wide_buf.push(0x41);
            wide_buf.push(0);
            wide_buf.push(0); wide_buf.push(0); wide_buf.push(0);
            wide_buf.extend_from_slice(&[0u8; 6]);
            wide_buf.push(0);
            wide_buf.extend_from_slice(&0u32.to_le_bytes());
        }
        let (cards2, mechanics2, wide_mulligan) = decode(&wide_buf).unwrap();
        let pip_weights2 = crate::types::compute_deck_pip_weights(&cards2);
        let mut rng2 = Rng::new(7);
        let rec_wide = run_game(&cards2, &mechanics2, &wide_mulligan, &pip_weights2, &mut rng2, 5);
        // Same deck, same seed — only the deck's configured Mana Base band changed
        // (max 4.0 -> 10.0): the same 7-land hand now fits inside the band, so it's kept.
        assert!(
            !rec_wide.took_mulligan(),
            "the same all-land hand should be kept once the band is widened to fit it"
        );
    }

    // ==================== New formula rules: multicolor lands, X-cost, MV4+ cap ====================
    //
    // These construct `SimCard`/`DeckPipWeights` directly rather than through the wire format —
    // `compute_deck_pip_weights`, `multicolor_land_multiplier`, and `MulliganConfig::score` are
    // pure functions over already-decoded types, so there's nothing wire-format-specific to
    // exercise here (that's covered separately by the mulligan-header decode tests above).

    fn make_sim_card(flags: u8, cmc: u8, color_mask: u8, mana_w: u8, mana_u: u8, mana_b: u8, mana_r: u8, mana_g: u8) -> crate::types::SimCard {
        crate::types::SimCard {
            flags,
            cmc,
            power: 0,
            toughness: 0,
            color_mask,
            mana_w,
            mana_u,
            mana_b,
            mana_r,
            mana_g,
            mana_generic: 0,
            formation_role: 0,
            mechanic_mask: 0,
        }
    }

    #[test]
    fn test_compute_deck_pip_weights_skews_toward_heavier_color() {
        let cards = vec![
            make_sim_card(0, 2, 0, 0, 0, 0, 3, 0), // non-land, 3 red pips
            make_sim_card(0, 1, 0, 0, 1, 0, 0, 0), // non-land, 1 blue pip
            make_sim_card(0x01, 0, 0x08, 0, 0, 0, 0, 0), // a land — its own pips are ignored
        ];
        let weights = crate::types::compute_deck_pip_weights(&cards);
        assert!((weights.r - 0.75).abs() < 1e-6, "expected r=0.75, got {}", weights.r);
        assert!((weights.u - 0.25).abs() < 1e-6, "expected u=0.25, got {}", weights.u);
        assert_eq!(weights.w, 0.0);
    }

    #[test]
    fn test_compute_deck_pip_weights_all_zero_when_no_colored_pips() {
        let cards = vec![make_sim_card(0x01, 0, 0x20, 0, 0, 0, 0, 0)]; // one colorless land
        let weights = crate::types::compute_deck_pip_weights(&cards);
        assert_eq!(weights.w, 0.0);
        assert_eq!(weights.u, 0.0);
        assert_eq!(weights.g, 0.0);
    }

    #[test]
    fn test_multicolor_land_multiplier_mono_color_unaffected() {
        let weights = crate::types::DeckPipWeights { w: 0.0, u: 1.0, b: 0.0, r: 0.0, g: 0.0 };
        // color_mask 0x02 = produces U only
        assert_eq!(crate::types::multicolor_land_multiplier(0x02, &weights), 1.0);
    }

    #[test]
    fn test_multicolor_land_multiplier_full_coverage_hits_cap() {
        let weights = crate::types::DeckPipWeights { w: 0.6, u: 0.4, b: 0.0, r: 0.0, g: 0.0 };
        // color_mask 0x01 | 0x02 = produces W and U, covering 100% of the deck's pips
        let multiplier = crate::types::multicolor_land_multiplier(0x01 | 0x02, &weights);
        assert!((multiplier - 1.4).abs() < 1e-6, "expected 1.4, got {}", multiplier);
    }

    #[test]
    fn test_multicolor_land_multiplier_barely_used_splash_stays_near_one() {
        let weights = crate::types::DeckPipWeights { w: 0.95, u: 0.05, b: 0.0, r: 0.0, g: 0.0 };
        let multiplier = crate::types::multicolor_land_multiplier(0x01 | 0x02, &weights);
        assert!((multiplier - 1.4).abs() < 1e-6, "covers 100% of pips (w+u) so still hits the cap");
        // A land covering only the barely-used color plus one totally-unused one should be low.
        let weights2 = crate::types::DeckPipWeights { w: 0.95, u: 0.05, b: 0.0, r: 0.0, g: 0.0 };
        let low_multiplier = crate::types::multicolor_land_multiplier(0x02 | 0x04, &weights2); // U+B
        assert!(low_multiplier < 1.1, "expected close to 1.0, got {}", low_multiplier);
    }

    #[test]
    fn test_score_x_cost_shifts_effective_mana_value() {
        let config = crate::types::MulliganConfig::default_config();
        let weights = crate::types::DeckPipWeights::default();

        let plain = make_sim_card(0, 0, 0, 0, 0, 0, 0, 0); // non-land, cmc 0, no X
        let x_cost = make_sim_card(0x20, 0, 0, 0, 0, 0, 0, 0); // non-land, cmc 0, X flag set

        assert_eq!(config.score(&plain, &weights), config.mv_values[0]);
        assert_eq!(config.score(&x_cost, &weights), config.mv_values[2]); // 0 + 2 = mv2
    }

    #[test]
    fn test_score_land_applies_multicolor_multiplier() {
        let config = crate::types::MulliganConfig::default_config();
        let weights = crate::types::DeckPipWeights { w: 0.5, u: 0.5, b: 0.0, r: 0.0, g: 0.0 };

        // is_land flag (0x01), produces W+U (color_mask 0x01 | 0x02)
        let dual_land = make_sim_card(0x01, 0, 0x01 | 0x02, 0, 0, 0, 0, 0);
        let value = config.score(&dual_land, &weights);
        assert!((value - config.land_value * 1.4).abs() < 1e-6);
    }

    #[test]
    fn test_default_config_caps_mv4_plus_at_point_two() {
        let config = crate::types::MulliganConfig::default_config();
        assert_eq!(config.mv_values[4], 0.2);
        assert_eq!(config.mv_values[5], 0.2);
        assert_eq!(config.mv_values[6], 0.2);
        assert_eq!(config.mv_values[7], 0.2);
    }

    // ==================== Mana Base (mana_base_value) ====================

    #[test]
    fn test_mana_base_value_basic_land() {
        let weights = crate::types::DeckPipWeights::default();
        let basic = make_sim_card(0x01 | 0x40, 0, 0, 0, 0, 0, 0, 0); // isLand + isBasicLand
        assert_eq!(crate::types::mana_base_value(&basic, &weights), 1.0);
    }

    #[test]
    fn test_mana_base_value_tapped_nonbasic_mono_land() {
        let weights = crate::types::DeckPipWeights::default();
        // isLand + entersTappedOrConditional, mono-U (color_mask 0x02)
        let tapped = make_sim_card(0x01 | 0x80, 0, 0x02, 0, 0, 0, 0, 0);
        let value = crate::types::mana_base_value(&tapped, &weights);
        assert!((value - 0.8).abs() < 1e-6, "expected 0.8, got {}", value);
    }

    #[test]
    fn test_mana_base_value_untapped_nonbasic_mono_land() {
        let weights = crate::types::DeckPipWeights::default();
        // isLand only, mono-U, no basic/tapped flags — a normal untapped nonbasic mono land
        let untapped = make_sim_card(0x01, 0, 0x02, 0, 0, 0, 0, 0);
        assert_eq!(crate::types::mana_base_value(&untapped, &weights), 1.0);
    }

    #[test]
    fn test_mana_base_value_multicolor_land_uses_multiplier() {
        let weights = crate::types::DeckPipWeights { w: 0.5, u: 0.5, b: 0.0, r: 0.0, g: 0.0 };
        let dual = make_sim_card(0x01, 0, 0x01 | 0x02, 0, 0, 0, 0, 0); // isLand, produces W+U
        let value = crate::types::mana_base_value(&dual, &weights);
        assert!((value - 1.4).abs() < 1e-6, "expected 1.4, got {}", value);
    }

    #[test]
    fn test_mana_base_value_cheap_mana_rock_by_cmc() {
        let weights = crate::types::DeckPipWeights::default();
        let rock_at = |cmc: u8| {
            let card = make_sim_card(0x08, cmc, 0, 0, 0, 0, 0, 0); // isManaProducing, non-land
            crate::types::mana_base_value(&card, &weights)
        };
        assert_eq!(rock_at(0), 1.0);
        assert_eq!(rock_at(1), 0.9);
        assert_eq!(rock_at(2), 0.6);
        assert_eq!(rock_at(3), 0.0);
    }

    #[test]
    fn test_mana_base_value_plain_non_mana_card_scores_zero() {
        let weights = crate::types::DeckPipWeights::default();
        let plain = make_sim_card(0x02, 2, 0, 0, 0, 0, 0, 0); // isCreature, no mana ability
        assert_eq!(crate::types::mana_base_value(&plain, &weights), 0.0);
    }
}
