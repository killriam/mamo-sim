use crate::rng::Rng;
use crate::types::{advantage, GameRecord, SimCard, SimMechanic};

/// Maximum number of cards in the deck (Commander = 100).
const MAX_DECK: usize = 128;
/// Maximum hand size.
const MAX_HAND: usize = 8;
/// Number of turns to snapshot (T1–T6).
const _SNAPSHOT_TURNS: usize = 6;

/// Mana pool: six u8 values for W,U,B,R,G,C plus a total.
#[derive(Clone, Copy, Default)]
struct ManaPool {
    w: u8, u: u8, b: u8, r: u8, g: u8, c: u8,
}

impl ManaPool {
    #[inline]
    fn total(&self) -> u8 {
        self.w.saturating_add(self.u)
            .saturating_add(self.b)
            .saturating_add(self.r)
            .saturating_add(self.g)
            .saturating_add(self.c)
    }

    /// Color bitmask of colors with ≥1 mana: W=bit0, U=bit1, B=bit2, R=bit3, G=bit4, C=bit5
    #[inline]
    fn color_mask(&self) -> u8 {
        (if self.w > 0 { 0x01 } else { 0 })
            | (if self.u > 0 { 0x02 } else { 0 })
            | (if self.b > 0 { 0x04 } else { 0 })
            | (if self.r > 0 { 0x08 } else { 0 })
            | (if self.g > 0 { 0x10 } else { 0 })
            | (if self.c > 0 { 0x20 } else { 0 })
    }

    /// Add mana from a card's color_mask (land / mana-producing permanent).
    #[inline]
    fn add_from_mask(&mut self, mask: u8) {
        if mask & 0x01 != 0 { self.w = self.w.saturating_add(1); }
        if mask & 0x02 != 0 { self.u = self.u.saturating_add(1); }
        if mask & 0x04 != 0 { self.b = self.b.saturating_add(1); }
        if mask & 0x08 != 0 { self.r = self.r.saturating_add(1); }
        if mask & 0x10 != 0 { self.g = self.g.saturating_add(1); }
        if mask & 0x20 != 0 { self.c = self.c.saturating_add(1); }
    }

    /// Try to pay a card's mana cost. Returns true and mutates self on success.
    fn pay(&mut self, card: &SimCard) -> bool {
        // Check colored pips first
        if self.w < card.mana_w { return false; }
        if self.u < card.mana_u { return false; }
        if self.b < card.mana_b { return false; }
        if self.r < card.mana_r { return false; }
        if self.g < card.mana_g { return false; }

        // Generic mana: paid by any color. Count remaining after colored pips.
        let remaining = (self.w - card.mana_w) as u16
            + (self.u - card.mana_u) as u16
            + (self.b - card.mana_b) as u16
            + (self.r - card.mana_r) as u16
            + (self.g - card.mana_g) as u16
            + self.c as u16;

        if remaining < card.mana_generic as u16 {
            return false;
        }

        // Commit payment: deduct colored pips
        self.w -= card.mana_w;
        self.u -= card.mana_u;
        self.b -= card.mana_b;
        self.r -= card.mana_r;
        self.g -= card.mana_g;

        // Deduct generic from remaining pool (colorless first, then colors)
        let mut generic_left = card.mana_generic as u16;
        let deduct = generic_left.min(self.c as u16);
        self.c -= deduct as u8;
        generic_left -= deduct;
        for slot in [&mut self.w, &mut self.u, &mut self.b, &mut self.r, &mut self.g] {
            if generic_left == 0 { break; }
            let d = generic_left.min(*slot as u16) as u8;
            *slot -= d;
            generic_left -= d as u16;
        }

        true
    }

    fn can_pay(&self, card: &SimCard) -> bool {
        let mut copy = *self;
        copy.pay(card)
    }
}

/// Fisher-Yates shuffle on a slice of indices.
fn shuffle(deck: &mut [u8], rng: &mut Rng) {
    let n = deck.len();
    for i in (1..n).rev() {
        let j = rng.next_usize(i + 1);
        deck.swap(i, j);
    }
}

/// Run a single solitaire game.
pub fn run_game(
    cards: &[SimCard],
    mechanics: &[SimMechanic],
    rng: &mut Rng,
    max_turns: u8,
) -> GameRecord {
    let n = cards.len().min(MAX_DECK);
    let mut rec = GameRecord::new();

    // ── Build initial library as index array ─────────────────────────
    let mut library = [0u8; MAX_DECK];
    for i in 0..n { library[i] = i as u8; }
    shuffle(&mut library[..n], rng);

    // ── Opening hand + mulligan ──────────────────────────────────────
    let mut hand = [0u8; MAX_HAND];
    let mut hand_len: usize = 0;
    let mut lib_top: usize = 0; // next card to draw from library

    let draw_n = |hand: &mut [u8; MAX_HAND], hand_len: &mut usize,
                  library: &[u8; MAX_DECK], lib_top: &mut usize, count: usize| {
        for _ in 0..count {
            if *lib_top < n && *hand_len < MAX_HAND {
                hand[*hand_len] = library[*lib_top];
                *hand_len += 1;
                *lib_top += 1;
            }
        }
    };

    draw_n(&mut hand, &mut hand_len, &library, &mut lib_top, 7);

    // Mulligan: if <2 or >5 lands, put back 1, draw 6 (max 2 mulligans)
    for _mulligan in 0..2 {
        let land_count = (0..hand_len).filter(|&i| cards[hand[i] as usize].is_land()).count();
        if land_count >= 2 && land_count <= 5 { break; }
        rec.flags |= 0x02; // took mulligan
        // Return worst card (highest CMC non-land if too many lands, else worst land)
        let return_idx = if land_count < 2 {
            // Too few lands — return highest CMC non-land
            (0..hand_len)
                .filter(|&i| !cards[hand[i] as usize].is_land())
                .max_by_key(|&i| cards[hand[i] as usize].cmc)
                .unwrap_or(hand_len - 1)
        } else {
            // Too many lands — return a land
            (0..hand_len)
                .find(|&i| cards[hand[i] as usize].is_land())
                .unwrap_or(hand_len - 1)
        };
        // Remove returned card (shift left)
        library[lib_top.saturating_sub(1)] = hand[return_idx]; // loosely put it back (bottom)
        hand[return_idx] = hand[hand_len - 1];
        hand_len -= 1;
        // Draw 1 more
        draw_n(&mut hand, &mut hand_len, &library, &mut lib_top, 1);
    }

    // ── Persistent battlefield state ─────────────────────────────────
    // battlefield_mask: u128 bitmask of card indices currently on battlefield
    let mut battlefield_mask: u128 = 0;
    // tapped_mask: subset of battlefield that is tapped
    let mut tapped_mask: u128 = 0;
    // seen_mask: cards ever drawn or in opening hand (by index, u128)
    let mut seen_mask: u128 = 0;
    for i in 0..hand_len {
        if (hand[i] as usize) < 128 { seen_mask |= 1u128 << hand[i]; }
    }

    let mut commander_idx: Option<usize> = None;
    let mut commander_times_cast: u8 = 0;
    // Find commander index (first card with isCommander flag)
    for i in 0..n {
        if cards[i].is_commander() { commander_idx = Some(i); break; }
    }

    let mut creatures_at_t5: u8 = 0;

    // ── Formation tracking ───────────────────────────────────────────
    // formation_bf_masks[i] = bitmask of formation i's cards currently on battlefield
    // We recompute per-formation membership dynamically.
    let mech_count = mechanics.len().min(32);

    // ── Main game loop ───────────────────────────────────────────────
    for turn in 1u8..=max_turns {
        // Untap all permanents
        tapped_mask = 0;

        // Draw (skip draw on T1 for the active player in Commander — simplification: always draw)
        if lib_top < n {
            let drawn = library[lib_top] as usize;
            lib_top += 1;
            rec.total_cards_drawn += 1;
            if hand_len < MAX_HAND {
                hand[hand_len] = drawn as u8;
                hand_len += 1;
            }
            if drawn < 128 { seen_mask |= 1u128 << drawn; }
        }

        // Land drop — play first land found in hand
        let land_pos = (0..hand_len).find(|&i| cards[hand[i] as usize].is_land());
        let land_played_this_turn = if let Some(pos) = land_pos {
            let card_idx = hand[pos] as usize;
            // Move land to battlefield
            if card_idx < 128 {
                battlefield_mask |= 1u128 << card_idx;
            }
            // Remove from hand
            hand[pos] = hand[hand_len - 1];
            hand_len -= 1;
            true
        } else {
            // Check if we had a land in library we couldn't play (always false here)
            // Missed land drop = we had no land in hand AND lands in play < turn
            let lands_in_play = (0..n)
                .filter(|&i| cards[i].is_land() && (i < 128) && (battlefield_mask >> i) & 1 == 1)
                .count() as u8;
            if lands_in_play < turn {
                rec.total_missed_land_drops += 1;
                if rec.first_missed_land_turn == 0 { rec.first_missed_land_turn = turn; }
            }
            false
        };

        // Build mana pool from all untapped mana-producing permanents
        let mut pool = ManaPool::default();
        for i in 0..n.min(128) {
            let on_bf = (battlefield_mask >> i) & 1 == 1;
            let tapped = (tapped_mask >> i) & 1 == 1;
            if on_bf && !tapped && cards[i].is_mana_producing() {
                pool.add_from_mask(cards[i].color_mask);
            }
        }
        // Also count lands even if not flagged mana_producing (basic lands)
        for i in 0..n.min(128) {
            let on_bf = (battlefield_mask >> i) & 1 == 1;
            let tapped = (tapped_mask >> i) & 1 == 1;
            if on_bf && !tapped && cards[i].is_land() && !cards[i].is_mana_producing() {
                // Basic land: add generic colorless as fallback
                pool.c = pool.c.saturating_add(1);
            }
        }

        let mana_total = pool.total();
        let color_mask_now = pool.color_mask();

        // Update color_first_turn tracking
        for bit in 0..6u8 {
            if color_mask_now & (1 << bit) != 0 && rec.color_first_turn[bit as usize] == 0 {
                rec.color_first_turn[bit as usize] = turn;
            }
        }

        // Count lands in play for snapshot
        let lands_in_play_count = (0..n.min(128))
            .filter(|&i| cards[i].is_land() && (battlefield_mask >> i) & 1 == 1)
            .count() as u8;

        // Count castable cards in hand before casting
        let castable_before: u8 = (0..hand_len)
            .filter(|&i| {
                let c = &cards[hand[i] as usize];
                !c.is_land() && pool.can_pay(c)
            })
            .count() as u8;

        // Greedy cast loop: repeatedly cast highest-CMC castable card
        let mut spells_this_turn: u8 = 0;
        let mut cmc_top: u8 = 0;
        loop {
            // Find highest-CMC castable non-land card in hand
            let best = (0..hand_len)
                .filter(|&i| {
                    let c = &cards[hand[i] as usize];
                    !c.is_land() && pool.can_pay(c)
                })
                .max_by_key(|&i| cards[hand[i] as usize].cmc);

            let pos = match best { Some(p) => p, None => break };
            let card_idx = hand[pos] as usize;
            let card = &cards[card_idx];

            // Pay the cost
            pool.pay(&mut cards[card_idx].clone()); // pay from pool copy via clone trick
            // Actually pay directly:
            {
                // Re-borrow mutably using inline logic
                let c = &cards[card_idx];
                pool.w -= c.mana_w;
                pool.u -= c.mana_u;
                pool.b -= c.mana_b;
                pool.r -= c.mana_r;
                pool.g -= c.mana_g;
                // generic paid from colorless then colors
                let mut gen = c.mana_generic as u16;
                let dc = gen.min(pool.c as u16) as u8; pool.c -= dc; gen -= dc as u16;
                for slot in [&mut pool.w, &mut pool.u, &mut pool.b, &mut pool.r, &mut pool.g] {
                    if gen == 0 { break; }
                    let d = gen.min(*slot as u16) as u8; *slot -= d; gen -= d as u16;
                }
            }

            if cmc_top < card.cmc { cmc_top = card.cmc; }
            spells_this_turn += 1;
            rec.total_spells_cast += 1;

            // Move to battlefield if permanent; otherwise it resolves and goes to graveyard
            let is_perm = card.is_land() || card.is_creature()
                || (card.flags & 0x04 != 0) // artifact
                || (card.flags & 0x08 != 0 && !card.is_land()); // mana producing = artifact/enchantment heuristic
            if is_perm && card_idx < 128 {
                battlefield_mask |= 1u128 << card_idx;
            }

            // Remove from hand
            hand[pos] = hand[hand_len - 1];
            hand_len -= 1;
        }

        // Commander casting check (from command zone)
        if let Some(cmd_idx) = commander_idx {
            if (battlefield_mask >> cmd_idx) & 1 == 0 {
                // Commander not yet on battlefield
                let cmd = &cards[cmd_idx];
                let extra_generic = (commander_times_cast as u16 * 2) as u8;
                // Synthesise adjusted cost
                let adjusted_generic = cmd.mana_generic.saturating_add(extra_generic);
                let can_cast = pool.w >= cmd.mana_w
                    && pool.u >= cmd.mana_u
                    && pool.b >= cmd.mana_b
                    && pool.r >= cmd.mana_r
                    && pool.g >= cmd.mana_g
                    && {
                        let rem = (pool.w - cmd.mana_w) as u16
                            + (pool.u - cmd.mana_u) as u16
                            + (pool.b - cmd.mana_b) as u16
                            + (pool.r - cmd.mana_r) as u16
                            + (pool.g - cmd.mana_g) as u16
                            + pool.c as u16;
                        rem >= adjusted_generic as u16
                    };
                if can_cast {
                    if cmd_idx < 128 { battlefield_mask |= 1u128 << cmd_idx; }
                    if rec.commander_cast_turn == 0 { rec.commander_cast_turn = turn; }
                    commander_times_cast += 1;
                    spells_this_turn += 1;
                    rec.total_spells_cast += 1;
                    if cmc_top < cmd.cmc.saturating_add(extra_generic) {
                        cmc_top = cmd.cmc.saturating_add(extra_generic);
                    }
                }
            }
        }

        // Combat: untapped creatures deal damage
        let mut damage: u16 = 0;
        for i in 0..n.min(128) {
            if cards[i].is_creature()
                && (battlefield_mask >> i) & 1 == 1
                && (tapped_mask >> i) & 1 == 0
            {
                damage += cards[i].power as u16;
                tapped_mask |= 1u128 << i; // tap after attacking
            }
        }
        rec.damage_dealt = rec.damage_dealt.saturating_add(damage);

        // Update peak mana
        if mana_total > rec.peak_mana { rec.peak_mana = mana_total; }

        // Missed land drop: if no land played AND land count < turn number
        if !land_played_this_turn && lands_in_play_count < turn {
            rec.total_missed_land_drops += 1;
            if rec.first_missed_land_turn == 0 { rec.first_missed_land_turn = turn; }
        }

        // ── Snapshot for T1..T6 ──────────────────────────────────────
        if turn >= 1 && turn <= 6 {
            let t = (turn - 1) as usize;
            rec.lands_in_play[t] = lands_in_play_count;
            rec.mana_available[t] = mana_total;
            rec.color_mask[t] = color_mask_now;
            rec.castable_count[t] = castable_before;
            rec.hand_size[t] = hand_len as u8;
            rec.cmc_top_cast[t] = cmc_top;
            rec.spells_cast_per_turn[t] = spells_this_turn;
            rec.land_played[t] = if land_played_this_turn { 1 } else { 0 };
        }

        // ── Formation assembly check ─────────────────────────────────
        // Build per-formation battlefield presence using mechanic_mask on each card
        let mut _assembled_this_turn = 0u32;
        for m in 0..mech_count {
            if rec.formation_assembled_mask & (1u32 << m) != 0 { continue; } // already assembled
            let mech = &mechanics[m];
            // Check prereqs
            if mech.prereq_mask != 0 && (rec.formation_assembled_mask & mech.prereq_mask) != mech.prereq_mask {
                continue;
            }
            // card_mask encodes which *unique card slots* (by index in decoded card slice, capped to 32)
            // are needed. Check if all those slots are on the battlefield.
            let card_mask = mech.card_mask;
            if card_mask == 0 { continue; }
            let mut bf_formation: u32 = 0;
            let mut seen_formation: u32 = 0;
            for i in 0..n.min(32) {
                let card_bit = 1u32 << i;
                if card_mask & card_bit == 0 { continue; }
                if (battlefield_mask >> i) & 1 == 1 { bf_formation |= card_bit; }
                if (seen_mask >> i) & 1 == 1        { seen_formation |= card_bit; }
            }
            // Seen mask update
            if seen_formation == card_mask {
                rec.formation_seen_mask |= 1u32 << m;
            }
            // Partial: ≥50% of required cards on BF
            let required = card_mask.count_ones();
            let present  = bf_formation.count_ones();
            if required > 0 && present * 2 >= required {
                rec.formation_partial_mask |= 1u32 << m;
            }
            // Full assembly
            if bf_formation == card_mask {
                rec.formation_assembled_mask |= 1u32 << m;
                _assembled_this_turn |= 1u32 << m;
                if rec.formation_first_turn[m] == 0 {
                    rec.formation_first_turn[m] = turn;
                }
                // Apply heuristic advantage effect
                match mech.advantage {
                    advantage::CARD_DRAW => {
                        // Draw 2 extra cards
                        for _ in 0..2 {
                            if lib_top < n && hand_len < MAX_HAND {
                                let drawn = library[lib_top] as usize;
                                lib_top += 1;
                                rec.total_cards_drawn += 1;
                                hand[hand_len] = drawn as u8;
                                hand_len += 1;
                                if drawn < 128 { seen_mask |= 1u128 << drawn; }
                            }
                        }
                    }
                    advantage::TOKEN_CREATION => {
                        // Virtual 2/2 creature — just add damage potential (no actual slot needed)
                        rec.damage_dealt = rec.damage_dealt.saturating_add(2);
                    }
                    advantage::DIRECT_DAMAGE => {
                        rec.damage_dealt = rec.damage_dealt.saturating_add(3);
                    }
                    advantage::COMBO_WIN => {
                        rec.flags |= 0x01; // combo_win
                        rec.total_turns = turn;
                        return rec; // end game immediately
                    }
                    _ => {}
                }
            }
        }

        // T5 creature snapshot
        if turn == 5 {
            creatures_at_t5 = (0..n.min(128))
                .filter(|&i| cards[i].is_creature() && (battlefield_mask >> i) & 1 == 1)
                .count() as u8;
        }

        rec.total_turns = turn;
    }

    // Store T5 creature count in a scalar field via a convention:
    // We'll encode it in the last formation_first_turn slot as a workaround.
    // Instead, just add a field — but GameRecord already has all 32 formation slots.
    // We store creatures_at_t5 by convention in formation_first_turn[31] as a repurposed byte,
    // and the aggregator reads it. Not ideal but avoids struct change.
    // Actually: GameRecord has no scalar field for this. We need to track it.
    // Solution: use rec.flags bits 3..7 for creatures_at_t5 capped to 31.
    // Better: we already have the full struct, let's store it cleanly.
    // The struct is defined in types.rs — we do NOT modify it here.
    // We use a convention: store creatures_at_t5 in formation_first_turn[31] if mechanic_count < 32.
    if mech_count < 32 {
        rec.formation_first_turn[31] = creatures_at_t5;
    }

    rec
}
