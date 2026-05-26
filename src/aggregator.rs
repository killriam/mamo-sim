use crate::types::GameRecord;
use std::collections::HashMap;

/// Compute median of a sorted vec.
fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() { return 0.0; }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn stddev(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 { return 0.0; }
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    var.sqrt()
}

/// Aggregate a slice of GameRecords into the full metric map.
/// All keys match the AggStats names expected by Evaluation.tsx where applicable.
pub fn aggregate(records: &[GameRecord], mech_keys: &[String]) -> HashMap<String, f64> {
    let n = records.len();
    if n == 0 { return HashMap::new(); }
    let mut out = HashMap::new();

    let nf = n as f64;

    // ── Scalar aggregates ────────────────────────────────────────────
    let turns_vec: Vec<f64> = records.iter().map(|r| r.total_turns as f64).collect();
    let mean_turns = turns_vec.iter().sum::<f64>() / nf;
    let mut sorted_turns = turns_vec.clone();
    sorted_turns.sort_by(|a, b| a.partial_cmp(b).unwrap());

    out.insert("avg_turns".into(), mean_turns);
    out.insert("median_turns".into(), median(&sorted_turns));
    out.insert("stdev_turns".into(), stddev(&turns_vec, mean_turns));

    let spells_vec: Vec<f64> = records.iter().map(|r| r.total_spells_cast as f64).collect();
    let _mean_spells = spells_vec.iter().sum::<f64>() / nf;
    let velocity_vec: Vec<f64> = records.iter().map(|r| {
        if r.total_turns > 0 { r.total_spells_cast as f64 / r.total_turns as f64 } else { 0.0 }
    }).collect();
    let mean_velocity = velocity_vec.iter().sum::<f64>() / nf;
    out.insert("avg_spell_velocity".into(), mean_velocity);
    out.insert("stdev_spell_velocity".into(), stddev(&velocity_vec, mean_velocity));

    let missed_vec: Vec<f64> = records.iter().map(|r| r.total_missed_land_drops as f64).collect();
    let mean_missed = missed_vec.iter().sum::<f64>() / nf;
    out.insert("avg_missed_land_drops".into(), mean_missed);
    out.insert("stdev_missed_land_drops".into(), stddev(&missed_vec, mean_missed));

    let first_miss_vec: Vec<f64> = records.iter()
        .filter(|r| r.first_missed_land_turn > 0)
        .map(|r| r.first_missed_land_turn as f64)
        .collect();
    out.insert("first_missed_land_turn".into(),
        if first_miss_vec.is_empty() { 0.0 }
        else { first_miss_vec.iter().sum::<f64>() / first_miss_vec.len() as f64 });

    let peak_vec: Vec<f64> = records.iter().map(|r| r.peak_mana as f64).collect();
    let mut sorted_peak = peak_vec.clone();
    sorted_peak.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out.insert("median_peak_mana".into(), median(&sorted_peak));

    let drawn_vec: Vec<f64> = records.iter().map(|r| r.total_cards_drawn as f64).collect();
    out.insert("avg_cards_drawn".into(), drawn_vec.iter().sum::<f64>() / nf);

    let dmg_vec: Vec<f64> = records.iter().map(|r| r.damage_dealt as f64).collect();
    let mean_dmg = dmg_vec.iter().sum::<f64>() / nf;
    out.insert("avg_damage_dealt".into(), mean_dmg);
    out.insert("stdev_damage_dealt".into(), stddev(&dmg_vec, mean_dmg));

    // Commander cast turn
    let cmd_turns: Vec<f64> = records.iter()
        .filter(|r| r.commander_cast_turn > 0)
        .map(|r| r.commander_cast_turn as f64)
        .collect();
    out.insert("avg_commander_castable_turn".into(),
        if cmd_turns.is_empty() { 0.0 }
        else { cmd_turns.iter().sum::<f64>() / cmd_turns.len() as f64 });

    // ── Focus 1: Land Drop Timeline ──────────────────────────────────
    for t in 0..6usize {
        let key_t = t + 1;
        out.insert(format!("land_in_play_t{}", key_t),
            records.iter().map(|r| r.lands_in_play[t] as f64).sum::<f64>() / nf);
        out.insert(format!("land_drop_rate_t{}", key_t),
            records.iter().filter(|r| r.land_played[t] == 1).count() as f64 / nf);
    }

    out.insert("avg_lands_played".into(),
        records.iter().map(|r| {
            r.land_played.iter().map(|&p| p as f64).sum::<f64>()
        }).sum::<f64>() / nf);

    // ── Focus 2: Mana per Turn + Color Availability ──────────────────
    for t in 0..6usize {
        let key_t = t + 1;
        out.insert(format!("mana_total_t{}", key_t),
            records.iter().map(|r| r.mana_available[t] as f64).sum::<f64>() / nf);

        // Color availability bitmask (W=0,U=1,B=2,R=3,G=4,C=5)
        let color_names = ["W", "U", "B", "R", "G", "C"];
        for bit in 0..6usize {
            let mask = 1u8 << bit;
            let rate = records.iter()
                .filter(|r| r.color_mask[t] & mask != 0)
                .count() as f64 / nf;
            out.insert(format!("color_{}_online_t{}", color_names[bit], key_t), rate);
        }
    }

    // Color first-turn averages
    let color_names = ["W", "U", "B", "R", "G", "C"];
    for bit in 0..6usize {
        let first_turns: Vec<f64> = records.iter()
            .filter(|r| r.color_first_turn[bit] > 0)
            .map(|r| r.color_first_turn[bit] as f64)
            .collect();
        out.insert(format!("color_{}_first_turn", color_names[bit]),
            if first_turns.is_empty() { 0.0 }
            else { first_turns.iter().sum::<f64>() / first_turns.len() as f64 });
    }

    // ── Focus 3: Castability per Turn ────────────────────────────────
    let mut total_stranded_turns: f64 = 0.0;
    for t in 0..6usize {
        let key_t = t + 1;
        out.insert(format!("castable_options_t{}", key_t),
            records.iter().map(|r| r.castable_count[t] as f64).sum::<f64>() / nf);

        let fractions: Vec<f64> = records.iter().map(|r| {
            if r.hand_size[t] == 0 { 0.0 }
            else { r.castable_count[t] as f64 / r.hand_size[t] as f64 }
        }).collect();
        out.insert(format!("castable_fraction_t{}", key_t),
            fractions.iter().sum::<f64>() / nf);

        let stranded_rate = records.iter()
            .filter(|r| r.castable_count[t] == 0 && r.hand_size[t] > 0)
            .count() as f64 / nf;
        out.insert(format!("stranded_pct_t{}", key_t), stranded_rate);
        total_stranded_turns += stranded_rate;
    }
    out.insert("avg_stranded_turns".into(), total_stranded_turns / 6.0);

    // ── Focus 4: CMC Drop Distribution ──────────────────────────────
    // drop_freq_cmc{1..7}: % of all turns across games where CMC-X was top cast
    let total_turns_all: f64 = records.iter().map(|r| r.total_turns as f64).sum();
    let mut cmc_freq = [0u32; 8]; // index = CMC (0 = no spell, 1..7)
    for r in records {
        for t in 0..6usize {
            let cmc = r.cmc_top_cast[t] as usize;
            if cmc > 0 && cmc < 8 {
                cmc_freq[cmc] += 1;
            }
        }
    }
    for cmc in 1..=7usize {
        out.insert(format!("drop_freq_cmc{}", cmc),
            if total_turns_all > 0.0 { cmc_freq[cmc] as f64 / total_turns_all } else { 0.0 });
    }

    // on_curve_rate_cmc{1..5}: % games where CMC-X spell cast on turn X exactly
    for cmc in 1..=5usize {
        let t = cmc - 1;
        let rate = records.iter()
            .filter(|r| r.cmc_top_cast[t] as usize >= cmc) // >= cmc cast on that turn
            .count() as f64 / nf;
        out.insert(format!("on_curve_rate_cmc{}", cmc), rate);
    }

    // avg_cmc_cast_t{1..5}
    for t in 0..5usize {
        let key_t = t + 1;
        out.insert(format!("avg_cmc_cast_t{}", key_t),
            records.iter().map(|r| r.cmc_top_cast[t] as f64).sum::<f64>() / nf);
    }

    // double_spell_rate_t{3..5}: % of turns T where ≥2 spells cast
    for t in 2..5usize { // 0-indexed: t=2→T3, t=3→T4, t=4→T5
        let key_t = t + 1;
        let rate = records.iter()
            .filter(|r| r.spells_cast_per_turn[t] >= 2)
            .count() as f64 / nf;
        out.insert(format!("double_spell_rate_t{}", key_t), rate);
    }

    // first_spell_turn
    let first_spell: Vec<f64> = records.iter().filter_map(|r| {
        (0..6).find(|&t| r.spells_cast_per_turn[t] > 0).map(|t| (t + 1) as f64)
    }).collect();
    out.insert("first_spell_turn".into(),
        if first_spell.is_empty() { 0.0 }
        else { first_spell.iter().sum::<f64>() / first_spell.len() as f64 });

    // ── Focus 5: Formation Completeness ─────────────────────────────
    let n_mech = mech_keys.len().min(32);

    let mut total_assembled: f64 = 0.0;
    let mut first_assembly_turns: Vec<f64> = Vec::new();
    let mut combo_wins: usize = 0;
    let mut partial_density_sum: f64 = 0.0;

    for m in 0..n_mech {
        let bit = 1u32 << m;
        let assembled_count = records.iter().filter(|r| r.formation_assembled_mask & bit != 0).count();
        let seen_count      = records.iter().filter(|r| r.formation_seen_mask & bit != 0).count();
        let partial_count   = records.iter().filter(|r| r.formation_partial_mask & bit != 0).count();

        let key = &mech_keys[m];
        out.insert(format!("formation_{}_assembled_pct", key), assembled_count as f64 / nf);
        out.insert(format!("formation_{}_seen_pct",      key), seen_count as f64 / nf);
        out.insert(format!("formation_{}_partial_pct",   key), partial_count as f64 / nf);

        let first_turns: Vec<f64> = records.iter()
            .filter(|r| r.formation_first_turn[m] > 0)
            .map(|r| r.formation_first_turn[m] as f64)
            .collect();
        let mean_first = if first_turns.is_empty() { 0.0 }
            else { first_turns.iter().sum::<f64>() / first_turns.len() as f64 };
        out.insert(format!("formation_{}_first_turn", key), mean_first);

        total_assembled += assembled_count as f64;
        partial_density_sum += partial_count as f64 / nf;

        // Collect first-assembly turns for avg_formation_assembly_turn
        first_assembly_turns.extend(first_turns.iter());
    }

    // combo_win_rate
    combo_wins = records.iter().filter(|r| r.combo_win()).count();
    out.insert("combo_win_rate".into(), combo_wins as f64 / nf);

    out.insert("avg_formations_assembled".into(),
        if n_mech > 0 { total_assembled / (nf * n_mech as f64) } else { 0.0 });

    out.insert("avg_formation_assembly_turn".into(),
        if first_assembly_turns.is_empty() { 0.0 }
        else { first_assembly_turns.iter().sum::<f64>() / first_assembly_turns.len() as f64 });

    out.insert("formation_density_t5".into(),
        if n_mech > 0 { partial_density_sum / n_mech as f64 } else { 0.0 });

    // Creatures at T5 (stored by convention in formation_first_turn[31])
    let creatures_t5: f64 = records.iter()
        .map(|r| r.formation_first_turn[31] as f64)
        .sum::<f64>() / nf;
    out.insert("avg_creatures_t5".into(), creatures_t5);

    out
}
