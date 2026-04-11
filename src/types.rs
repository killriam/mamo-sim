/// Decoded card — one entry per physical copy in the deck.
/// Fits in a cache line (16 bytes from wire format + padding).
#[derive(Clone, Copy)]
pub struct SimCard {
    /// Bit flags: 0=isLand, 1=isCreature, 2=isArtifact, 3=isManaProducing, 4=isCommander
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

    /// True if this card stays on the battlefield (permanent types)
    #[inline] pub fn is_permanent(&self) -> bool {
        // Land, Creature, Artifact, Enchantment, Planeswalker — everything except instants/sorceries
        // We approximate: lands + creatures + artifacts (flag bits 0,1,2) are permanents.
        // Everything else (spells) is non-permanent.
        self.flags & 0x07 != 0 || self.mana_generic == 0 // fallback: low-CMC non-creatures may be sorceries
    }
}

/// A mechanic group (formation) stripped to simulation essentials.
#[derive(Clone, Copy)]
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
    #[inline] pub fn took_mulligan(&self) -> bool { self.flags & 0x02 != 0 }
}
