/// Decoded card — one entry per physical copy in the deck.
/// Fits in a cache line (16 bytes from wire format + padding).
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct SimCard {
    /// Bit flags: 0=isLand, 1=isCreature, 2=isArtifact, 3=isManaProducing, 4=isCommander,
    /// 5=hasXCost (mulligan-scoring only — never affects `cmc`, which stays the real mana
    /// value used everywhere else: castability, land-drop tracking, `is_permanent()`, etc.)
    pub flags: u8,
    /// Converted mana cost (capped at 15)
    pub cmc: u8,
    /// Creature power ('*' encoded as 3)
    pub power: u8,
    pub toughness: u8,
    /// Bitmask of mana colors this card produces when tapped: W=0,U=1,B=2,R=3,G=4,C=5
    pub color_mask: u8,
    /// Colored mana pips in cost
    pub mana_w: u8,
    pub mana_u: u8,
    pub mana_b: u8,
    pub mana_r: u8,
    pub mana_g: u8,
    pub mana_generic: u8,
    /// FormationRoleType ordinal
    pub formation_role: u8,
    /// Bitmask: bit i = card belongs to mechanic group i (max 32 groups)
    pub mechanic_mask: u32,
}

impl SimCard {
    #[inline] pub fn is_land(&self)          -> bool { self.flags & 0x01 != 0 }
    #[inline] pub fn is_creature(&self)      -> bool { self.flags & 0x02 != 0 }
    #[inline] pub fn is_mana_producing(&self)-> bool { self.flags & 0x08 != 0 }
    #[inline] pub fn is_commander(&self)     -> bool { self.flags & 0x10 != 0 }
    #[inline] pub fn is_x_cost(&self)        -> bool { self.flags & 0x20 != 0 }

    /// True if this card stays on the battlefield (permanent types)
    #[allow(dead_code)]
    #[inline] pub fn is_permanent(&self) -> bool {
        // Land, Creature, Artifact, Enchantment, Planeswalker — everything except instants/sorceries
        // We approximate: lands + creatures + artifacts (flag bits 0,1,2) are permanents.
        // Everything else (spells) is non-permanent.
        self.flags & 0x07 != 0 || self.mana_generic == 0 // fallback: low-CMC non-creatures may be sorceries
    }
}

/// Deck-specific mulligan scoring, decoded from the wire format's header.
/// Mirrors MaMoFrontend's `computeHandScore`/`MulliganConfig`
/// (src/components/scenario/MulliganValueEditor.tsx, src/types/scenario.ts) and Forge's
/// `forge.ai.mulligan.DecklistMulliganEvaluator` — the three places that score an opening
/// hand the same way, kept in sync by convention rather than a shared type across languages.
pub const MAX_MULLIGAN_THRESHOLDS: usize = 4;

#[derive(Clone, Copy)]
pub struct MulliganConfig {
    pub land_value: f32,
    /// Standard value curve, indexed by exact mana value: index 0-6 = mana value 0-6,
    /// index 7 = mana value 7+. Mirrors MaMoFrontend's `MulliganCardValues`/`cardValueFromCurve`.
    pub mv_values: [f32; 8],
    /// (round, min_value) pairs; only the first `threshold_count` entries are valid.
    pub thresholds: [(u8, f32); MAX_MULLIGAN_THRESHOLDS],
    pub threshold_count: u8,
}

/// Deck-wide colored-pip weight per color: each color's share (0.0-1.0, summing to 1 across
/// all five) of the deck's total colored mana pips in non-land cards. All-zero if the deck has
/// no colored pips at all. Mirrors MaMoFrontend's `DeckPipWeights`/`computeDeckPipWeights`
/// (`MulliganValueEditor.tsx`) — used to score how well a multicolor land's color-fixing
/// matches what the deck actually needs.
#[derive(Clone, Copy, Default)]
pub struct DeckPipWeights {
    pub w: f32,
    pub u: f32,
    pub b: f32,
    pub r: f32,
    pub g: f32,
}

/// Computed once per batch (all games in a batch share the same deck), not per game — an
/// O(games × cards) redundant pass would otherwise redo identical work on every simulated game.
pub fn compute_deck_pip_weights(cards: &[SimCard]) -> DeckPipWeights {
    let mut total_w = 0u32;
    let mut total_u = 0u32;
    let mut total_b = 0u32;
    let mut total_r = 0u32;
    let mut total_g = 0u32;

    for card in cards {
        if card.is_land() {
            continue;
        }
        total_w += card.mana_w as u32;
        total_u += card.mana_u as u32;
        total_b += card.mana_b as u32;
        total_r += card.mana_r as u32;
        total_g += card.mana_g as u32;
    }

    let total = (total_w + total_u + total_b + total_r + total_g) as f32;
    if total == 0.0 {
        return DeckPipWeights::default();
    }

    DeckPipWeights {
        w: total_w as f32 / total,
        u: total_u as f32 / total,
        b: total_b as f32 / total,
        r: total_r as f32 / total,
        g: total_g as f32 / total,
    }
}

/// Value multiplier for a land producing the colors in `color_mask`, from 1.0 (mono-color/
/// colorless — unaffected) up to 1.4 (produces every color the deck's pips are weighted
/// toward). Mirrors MaMoFrontend's `multicolorLandMultiplier`. Uses the same `color_mask` bit
/// convention as everywhere else in this crate: W=0x01, U=0x02, B=0x04, R=0x08, G=0x10.
pub fn multicolor_land_multiplier(color_mask: u8, pip_weights: &DeckPipWeights) -> f32 {
    let mut producing_count = 0u8;
    let mut coverage = 0.0f32;
    if color_mask & 0x01 != 0 { producing_count += 1; coverage += pip_weights.w; }
    if color_mask & 0x02 != 0 { producing_count += 1; coverage += pip_weights.u; }
    if color_mask & 0x04 != 0 { producing_count += 1; coverage += pip_weights.b; }
    if color_mask & 0x08 != 0 { producing_count += 1; coverage += pip_weights.r; }
    if color_mask & 0x10 != 0 { producing_count += 1; coverage += pip_weights.g; }
    if producing_count < 2 {
        return 1.0;
    }
    1.0 + 0.4 * coverage.min(1.0)
}

impl MulliganConfig {
    /// Matches MaMoFrontend's `DEFAULT_MULLIGAN_CONFIG` — used both as the simulator's
    /// built-in default and as the fallback for any round a caller didn't configure.
    pub fn default_config() -> Self {
        MulliganConfig {
            land_value: 1.0,
            // mv4+ capped at 0.2: mana value 4+ is worth meaningfully less to see in an
            // opening hand.
            mv_values: [0.85, 0.8, 0.75, 0.6, 0.2, 0.2, 0.2, 0.2],
            thresholds: [(0, 3.5), (1, 3.0), (2, 2.5), (3, 2.0)],
            threshold_count: 4,
        }
    }

    /// Scores a single card: X-cost non-lands (mulligan scoring only — `card.cmc` itself, used
    /// everywhere else, is untouched) count as +2 mana value before the curve lookup; lands
    /// producing 2+ colors are boosted by `multicolor_land_multiplier`.
    #[inline]
    pub fn score(&self, card: &SimCard, pip_weights: &DeckPipWeights) -> f32 {
        if card.is_land() {
            self.land_value * multicolor_land_multiplier(card.color_mask, pip_weights)
        } else {
            let effective_cmc = if card.is_x_cost() {
                (card.cmc as u16 + 2).min(7) as u8
            } else {
                card.cmc.min(7)
            };
            self.mv_values[effective_cmc as usize]
        }
    }

    /// Minimum hand value required to keep, for the given mulligan round (0 = initial 7-card
    /// hand). Falls back to `default_config()`'s threshold for that round if unconfigured.
    pub fn min_value_for_round(&self, round: u8) -> f32 {
        for i in 0..self.threshold_count as usize {
            if self.thresholds[i].0 == round {
                return self.thresholds[i].1;
            }
        }
        let default = Self::default_config();
        for i in 0..default.threshold_count as usize {
            if default.thresholds[i].0 == round {
                return default.thresholds[i].1;
            }
        }
        0.0
    }
}

/// A mechanic group (formation) stripped to simulation essentials.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct SimMechanic {
    /// ActivationConditionType ordinal
    pub activation: u8,
    /// AdvantageOutputType ordinal — used to determine effect on assembly
    pub advantage: u8,
    /// EvaluationDimension ordinal
    pub dimension: u8,
    /// Bitmask of card *indices* (in the expanded deck) required on battlefield simultaneously.
    /// Because deck is 100 cards but indices repeat per copy, we store *unique card slot* masks.
    /// This is recomputed from oracle-ID matching during codec decode.
    pub card_mask: u32,
    /// Bitmask of mechanic group indices that must be assembled before this one can fire.
    pub prereq_mask: u32,
}

/// AdvantageOutputType ordinals that matter for simulation effects.
#[allow(dead_code)]
pub mod advantage {
    pub const CARD_DRAW: u8       = 1;
    pub const MANA_GENERATION: u8 = 2;
    pub const TOKEN_CREATION: u8  = 3;
    pub const DIRECT_DAMAGE: u8   = 4;
    pub const COMBO_WIN: u8       = 5;
}

/// Per-game statistics. Stack-allocated, ~103 bytes, no heap use.
pub struct GameRecord {
    // ── Turn snapshots T1..T6 (index = turn - 1) ──────────────────────
    /// Cumulative lands in play at end of each turn
    pub lands_in_play: [u8; 6],
    /// Total mana pool size before spending on that turn
    pub mana_available: [u8; 6],
    /// WUBRGC color bitmask of mana sources available on that turn
    pub color_mask: [u8; 6],
    /// Count of castable (non-land) cards in hand at start of main phase
    pub castable_count: [u8; 6],
    /// Hand size at start of main phase (before casting)
    pub hand_size: [u8; 6],
    /// CMC of the highest-CMC spell cast this turn (0 = nothing cast)
    pub cmc_top_cast: [u8; 6],
    /// Count of spells cast this turn
    pub spells_cast_per_turn: [u8; 6],
    /// 1 if a land was played on this turn, 0 otherwise
    pub land_played: [u8; 6],

    // ── Color first-available (W=0,U=1,B=2,R=3,G=4,C=5) ───────────────
    /// Turn number when each color first appeared in the mana pool (0 = never)
    pub color_first_turn: [u8; 6],

    // ── Scalar game outcomes ───────────────────────────────────────────
    pub total_turns: u8,
    pub total_spells_cast: u8,
    pub total_missed_land_drops: u8,
    /// Turn of first missed land drop (0 = never missed)
    pub first_missed_land_turn: u8,
    pub peak_mana: u8,
    pub total_cards_drawn: u8,
    pub damage_dealt: u16,
    /// Turn commander was first cast (0 = never)
    pub commander_cast_turn: u8,

    // ── Formation tracking (max 32 formations) ─────────────────────────
    /// Bit i = formation i was fully assembled on the battlefield simultaneously
    pub formation_assembled_mask: u32,
    /// Bit i = all cards for formation i were *seen* (hand or BF) at any point
    pub formation_seen_mask: u32,
    /// Bit i = ≥50% of formation i's cards appeared on BF at same time
    pub formation_partial_mask: u32,
    /// Turn formation i was first fully assembled (0 = never). Max 32 formations.
    pub formation_first_turn: [u8; 32],

    // ── Flags ──────────────────────────────────────────────────────────
    /// bit 0 = combo_win, bit 1 = took a mulligan, bit 2 = opened with 0 lands
    pub flags: u8,
}

impl GameRecord {
    pub fn new() -> Self {
        GameRecord {
            lands_in_play: [0; 6],
            mana_available: [0; 6],
            color_mask: [0; 6],
            castable_count: [0; 6],
            hand_size: [0; 6],
            cmc_top_cast: [0; 6],
            spells_cast_per_turn: [0; 6],
            land_played: [0; 6],
            color_first_turn: [0; 6],
            total_turns: 0,
            total_spells_cast: 0,
            total_missed_land_drops: 0,
            first_missed_land_turn: 0,
            peak_mana: 0,
            total_cards_drawn: 0,
            damage_dealt: 0,
            commander_cast_turn: 0,
            formation_assembled_mask: 0,
            formation_seen_mask: 0,
            formation_partial_mask: 0,
            formation_first_turn: [0; 32],
            flags: 0,
        }
    }

    #[inline] pub fn combo_win(&self) -> bool { self.flags & 0x01 != 0 }
    #[allow(dead_code)]
    #[inline] pub fn took_mulligan(&self) -> bool { self.flags & 0x02 != 0 }
}
