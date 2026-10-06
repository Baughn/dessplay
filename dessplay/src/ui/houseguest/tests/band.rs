//! The stillness band (phase 5c, D2 as amended by B5 and the round-3
//! rule): how much she moves on a fed afternoon ([`afternoon`]), per
//! census room and mood, quiet and at the room's chat cadence, each
//! condition judged on its own. Moving is the census's: moving in sight
//! after the warm-up ([`WARM_MS`]) as a share of her time in sight then,
//! pooled over the cell's visits (Σ moving ÷ Σ in sight), and her
//! set-offs a minute in sight.
//!
//! **The rule** (the user's, round 3): per mood, in every room and both
//! chat conditions, a share ceiling and a floor ("not dead") and a cap
//! on set-offs a minute ([`Band::of`]): a set-off draws the eye the same
//! in any room, so the cap is the mood's, not the room's. And per room,
//! industrious moves at least [`SPREAD`]× as much as lazy (full strength
//! only).
//!
//! **Line art only.** The houseguest's tests otherwise loop both drawing
//! modes; these don't, by design: the band is pinned in line art (the
//! user's client draws it), and the home and resident run the same in
//! either mode, seed for seed (phase5c/baseline.md: neither has text she
//! can reach), so a second mode would double the gate's cost to measure
//! the stage's ASCII alone, which the user's client doesn't draw.
//!
//! **Two strengths, one rule** (B5): every threshold stands at least 3σ
//! from the cell's aim ([`TUNED`]; while it's empty, the band's middle
//! and the mood's cap, with [`BASELINE`]'s σ), σ being that of the mean of the
//! visits run, and is never inside the band. With `CENSUS_BAND_SEEDS` at
//! full strength (the most any line asks for: each band line prints the
//! N from which its share bounds are the band itself and its cap the
//! mood's own; each spread line the N from which its floor is
//! [`SPREAD`]) the thresholds are the band's. Run at step boundaries:
//!
//! ```text
//! cargo nextest run -p dessplay --run-ignored all -E 'test(/houseguest::tests::band::/)'
//! CENSUS_BAND_SEEDS=<n> cargo nextest run -p dessplay --release --profile band \
//!   --run-ignored all -E 'test(/houseguest::tests::band::/)' --success-output final
//! ```
//!
//! (`--no-capture` would run them one at a time; `--profile band` gives
//! them minutes each.)
//!
//! **What the gate's seeds can see.** One visit's reading varies widely
//! (at step 8b's baseline, a stage visit read anywhere from a few % to
//! about 80% of its time moving), so at the gate's [`GATE_SEEDS`] 3σ
//! on the share is 3 × √2 × [`TUNED`]'s 4-visit σ (1.7–3.6): about 7
//! to 15 points. The gate
//! catches a gross excess of moving and of set-offs, no more, and a
//! "too still" regression all but never fails there. Full strength is
//! the real check: passing at the gate is not evidence the band is met
//! (the user, step 4: "I'll know to check them if something feels
//! off").
//!
//! **Printed beside it, not judged:** a windowed home, a TV-only home and
//! a resident with text at her floor's height (`census::printed_rooms`,
//! phase 5c step 11).
//!
//! **Tuned** in phase 5c step 8c ([`TUNED`]), the bare stage's floors
//! lowered by the user: every cell inside, but [`SHORT`]'s, whose tests
//! stay ignored with their numbers (phase5c/baseline.md, "Shipped (step
//! 8c)").

use super::census::{
    Room, Visit, WARM_MS, afternoon, at_afternoon, chat_name, furnished_room, resident_room,
    stage_room, with_chat,
};
use crate::ui::houseguest::brain::Mood;
use crate::ui::houseguest::osaka::longest_still_ms;
use crate::ui::houseguest::routine::Slot;
use dessplay_core::test_support::census_band_seeds;

/// How long each band visit runs (minutes): the warm-up and then twice
/// her longest still act (B5), so a visit's reading isn't one act's
/// luck. With the levers she ships with (phase 5c step 8c) that's her
/// window's chain, lazy: leaning on the sill (3 minutes, ×1.5), sitting
/// in front of it (40 s, ×1.5) and dozing or watching the clouds (60 s,
/// ×1.5), 7 minutes: 3 + 2 × 7, with no headroom.
/// [`band_visits_outlast_her_longest_still_act_twice`] holds it to that.
const BAND_MINUTES: u64 = 17;

/// The seeds a cell runs at the gate (`CENSUS_BAND_SEEDS` overrides):
/// what the stage, the costliest room, affords in a few seconds a test
/// (a 17-minute line-art visit costs about 1.3 s there under the gate's
/// opt-level 2, less in the home and the resident; a test runs two
/// conditions: about 4 s a stage test alone, more beside the gate's
/// other tests).
const GATE_SEEDS: u64 = 2;

/// The margin every threshold keeps from the aim, in σ of the cell's
/// mean (B5).
const SIGMAS: f64 = 3.0;

/// The visits a σ in [`BASELINE`] or [`TUNED`] is of the mean of.
const SET: f64 = 4.0;

/// The minutes [`BASELINE`]'s visits ran (step 8b's 9; [`BAND_MINUTES`]
/// is longer since step 8c, so [`shorter_visits`] is under 1, about
/// 0.65).
const BASELINE_MINUTES: u64 = 9;

/// The industrious ÷ lazy spread each room's moving keeps (B5).
const SPREAD: f64 = 1.6;

/// σ of a band visit's reading over a baseline visit's: a share or rate
/// read over less time in sight varies more, about as 1/√time (her acts
/// as the independent draws). It carries [`BASELINE`]'s σ to the band's
/// visits (1 while both are [`BAND_MINUTES`] long). Never applied to
/// [`TUNED`]'s, read at the band's own length.
fn shorter_visits() -> f64 {
    let warm = WARM_MS / 60_000;
    ((BASELINE_MINUTES - warm) as f64 / (BAND_MINUTES - warm) as f64).sqrt()
}

/// The census rooms by name, as their builders name them.
const ROOMS: [&str; 3] = ["stage", "home", "resident"];

/// A cell: room, mood, and whether the room is quiet.
type Cell = (&'static str, Mood, bool);

/// Every cell, room by room.
fn cells() -> impl Iterator<Item = Cell> {
    ROOMS.into_iter().flat_map(|room| {
        Mood::ALL
            .into_iter()
            .flat_map(move |mood| [true, false].map(|quiet| (room, mood, quiet)))
    })
}

/// A mood's band, the same in every room: moving in sight (% of her
/// time in sight) between `floor` and `ceiling`, and set-offs a minute
/// in sight at most `cap`.
#[derive(Clone, Copy)]
struct Band {
    floor: f64,
    ceiling: f64,
    cap: f64,
}

impl Band {
    /// The user's band (round 3): about 15 lazy, 20–25 ordinary and
    /// dreamy, up to 30 industrious, with ceilings a point or two over;
    /// floors where she'd read as dead; and a cap on set-offs a minute
    /// by mood, since a set-off draws the eye the same in any room.
    ///
    /// The bare stage's floors are lower (the user, step 8c): a room that
    /// keeps her things close (her made pieces and the lines she pulls on
    /// her own floor) moves her in shorter trips, so under the same cap
    /// on set-offs its share runs lower.
    fn of(room: &str, mood: Mood) -> Band {
        let stage = room == "stage";
        match mood {
            Mood::Lazy => Band {
                floor: if stage { 4.0 } else { 5.0 },
                ceiling: 17.0,
                cap: 1.5,
            },
            Mood::Ordinary | Mood::Dreamy => Band {
                floor: if stage { 6.0 } else { 8.0 },
                ceiling: 26.0,
                cap: 2.25,
            },
            Mood::Industrious => Band {
                floor: if stage { 9.0 } else { 12.0 },
                ceiling: 31.0,
                cap: 3.0,
            },
        }
    }

    /// Its middle, which the tuning aims at (B5: "tune to the middle").
    fn middle(self) -> f64 {
        (self.floor + self.ceiling) / 2.0
    }

    /// Whether `share` lies strictly inside it.
    fn holds(self, share: f64) -> bool {
        self.floor < share && share < self.ceiling
    }

    /// How far `aim` stands inside it, from the nearer edge: the most 3σ
    /// may be for the band itself to bound a cell aimed there.
    fn room(self, aim: f64) -> f64 {
        (aim - self.floor).min(self.ceiling - aim)
    }
}

/// A cell's moving in sight (%) and set-offs a minute in sight, each
/// with σ of a mean of [`SET`] band visits ([`BAND_MINUTES`] long).
#[derive(Clone, Copy)]
struct Stats {
    share: f64,
    share_sd: f64,
    rate: f64,
    rate_sd: f64,
}

/// The baseline: step 7's state (the levers neutral) with step 8's
/// blink and credit fix, at the driver that delivers each chat line at
/// its own time (phase5c/baseline.md, "Step 8b: the retune, stopped",
/// c0), [`BASELINE_MINUTES`] long, line art, 20 sets × 4 seeds: moving
/// %, σ of a 4-visit mean, set-offs a minute, its σ. The home's and
/// resident's rows stand for ASCII too on step 3's finding that those
/// rooms don't depend on drawing mode (no text in her reach there).
// Measured numbers: none is a constant of nature.
#[allow(clippy::approx_constant)]
const BASELINE: [(&str, Mood, bool, [f64; 4]); 24] = [
    ("stage", Mood::Ordinary, true, [30.6, 6.47, 4.87, 0.59]),
    ("stage", Mood::Ordinary, false, [29.4, 7.87, 5.14, 0.49]),
    ("stage", Mood::Lazy, true, [21.7, 7.23, 4.18, 0.42]),
    ("stage", Mood::Lazy, false, [24.8, 8.00, 4.41, 0.43]),
    ("stage", Mood::Industrious, true, [33.0, 6.06, 5.07, 0.47]),
    ("stage", Mood::Industrious, false, [36.7, 7.16, 5.45, 0.62]),
    ("stage", Mood::Dreamy, true, [27.2, 6.51, 4.69, 0.55]),
    ("stage", Mood::Dreamy, false, [30.5, 7.06, 4.95, 0.66]),
    ("home", Mood::Ordinary, true, [48.0, 5.99, 2.28, 0.18]),
    ("home", Mood::Ordinary, false, [43.4, 5.40, 2.42, 0.21]),
    ("home", Mood::Lazy, true, [35.7, 4.47, 1.55, 0.21]),
    ("home", Mood::Lazy, false, [33.9, 5.83, 1.74, 0.22]),
    ("home", Mood::Industrious, true, [52.8, 4.42, 2.73, 0.25]),
    ("home", Mood::Industrious, false, [51.4, 5.05, 2.96, 0.25]),
    ("home", Mood::Dreamy, true, [46.2, 4.31, 2.29, 0.16]),
    ("home", Mood::Dreamy, false, [42.1, 6.02, 2.35, 0.20]),
    ("resident", Mood::Ordinary, true, [38.8, 3.14, 2.82, 0.17]),
    ("resident", Mood::Ordinary, false, [35.3, 5.34, 3.03, 0.28]),
    ("resident", Mood::Lazy, true, [33.8, 4.15, 2.45, 0.17]),
    ("resident", Mood::Lazy, false, [31.9, 4.46, 2.58, 0.17]),
    (
        "resident",
        Mood::Industrious,
        true,
        [41.5, 4.26, 2.84, 0.15],
    ),
    (
        "resident",
        Mood::Industrious,
        false,
        [38.3, 4.05, 3.02, 0.22],
    ),
    ("resident", Mood::Dreamy, true, [36.6, 5.71, 2.64, 0.22]),
    ("resident", Mood::Dreamy, false, [31.8, 3.69, 2.81, 0.21]),
];

/// The tuned cells, which a tuning fills from one tuned census read at
/// [`BAND_MINUTES`] (`CENSUS_MINUTES=<BAND_MINUTES> CENSUS_MODES=line
/// fed_afternoon_census`, line art): moving %, σ of a 4-visit mean,
/// set-offs a minute, its σ, by room, mood and quiet. Empty, or every
/// cell once ([`band_cells_are_whole_and_tuned_inside_the_band`] holds
/// it, and that each aim is inside its band, under its mood's cap, and
/// spread over [`SPREAD`] a room): while empty, a cell aims at its
/// band's middle and its mood's cap, with the baseline's σ.
///
/// Step 8c's, from what ships (`CENSUS_MINUTES=17 CENSUS_ROOMS=band
/// CENSUS_MODES=line fed_afternoon_census`, 20 sets × 4 seeds, release;
/// phase5c/baseline.md, "Shipped (step 8c)").
// Measured numbers: none is a constant of nature.
#[allow(clippy::approx_constant)]
const TUNED: [(&str, Mood, bool, [f64; 4]); 24] = [
    ("stage", Mood::Ordinary, true, [7.2, 2.53, 1.58, 0.18]),
    ("stage", Mood::Ordinary, false, [6.8, 2.53, 1.69, 0.28]),
    ("stage", Mood::Lazy, true, [6.5, 1.98, 1.20, 0.18]),
    ("stage", Mood::Lazy, false, [6.6, 2.54, 1.31, 0.20]),
    ("stage", Mood::Industrious, true, [9.7, 2.19, 2.21, 0.45]),
    ("stage", Mood::Industrious, false, [8.2, 2.26, 2.18, 0.41]),
    ("stage", Mood::Dreamy, true, [7.0, 2.32, 1.60, 0.19]),
    ("stage", Mood::Dreamy, false, [6.9, 2.64, 1.71, 0.23]),
    ("home", Mood::Ordinary, true, [21.8, 2.18, 1.23, 0.10]),
    ("home", Mood::Ordinary, false, [21.0, 2.76, 1.34, 0.09]),
    ("home", Mood::Lazy, true, [11.8, 1.74, 0.65, 0.08]),
    ("home", Mood::Lazy, false, [11.9, 2.03, 0.71, 0.07]),
    ("home", Mood::Industrious, true, [29.9, 3.63, 1.71, 0.16]),
    ("home", Mood::Industrious, false, [28.8, 3.14, 1.90, 0.16]),
    ("home", Mood::Dreamy, true, [21.1, 2.63, 1.19, 0.08]),
    ("home", Mood::Dreamy, false, [21.9, 2.11, 1.38, 0.10]),
    ("resident", Mood::Ordinary, true, [9.8, 1.80, 1.08, 0.10]),
    ("resident", Mood::Ordinary, false, [10.0, 2.21, 1.21, 0.11]),
    ("resident", Mood::Lazy, true, [6.0, 2.43, 0.54, 0.10]),
    ("resident", Mood::Lazy, false, [6.3, 2.18, 0.68, 0.10]),
    (
        "resident",
        Mood::Industrious,
        true,
        [15.9, 2.51, 1.35, 0.11],
    ),
    (
        "resident",
        Mood::Industrious,
        false,
        [14.5, 2.08, 1.46, 0.10],
    ),
    ("resident", Mood::Dreamy, true, [9.2, 2.26, 1.04, 0.11]),
    ("resident", Mood::Dreamy, false, [8.6, 2.29, 1.11, 0.09]),
];

/// A room's cell (mood, quiet) or, with `None`, its spread, short of
/// the band, and why.
type Short = (&'static str, Option<(Mood, bool)>, &'static str);

/// What the tuning left outside the band (the user, step 8c: "ship it";
/// phase5c/baseline.md, "Shipped (step 8c)"): a cell (room, mood, quiet)
/// or a room's spread (room, `None`), with why. Its band or spread test
/// stays ignored, its bounds are the band's own (an aim outside the band
/// never widens them), and the aims' checks pass it over.
const SHORT: [Short; 2] = [
    (
        "stage",
        Some((Mood::Industrious, false)),
        "8.4% with chat against the stage's industrious floor of 9 at full strength (N = 525; quiet 9.1): \
         an industrious linger of 0.7 brings it to 9.2 but the home's to 32.4 against 31 (80 visits)",
    ),
    (
        "stage",
        None,
        "industrious ÷ lazy 1.39 against 1.6 at full strength (8.9 ÷ 6.4, N = 200): her moods \
         read alike on the floor and in what she made (about 17% reading on her back in each)",
    ),
];

/// Why `cell` is short of the band, if it is.
fn short(cell: Cell) -> Option<&'static str> {
    SHORT
        .iter()
        .find(|&&(room, at, _)| room == cell.0 && at == Some((cell.1, cell.2)))
        .map(|&(.., why)| why)
}

/// Why `room`'s spread is short of [`SPREAD`], if it is.
fn short_spread(room: &str) -> Option<&'static str> {
    SHORT
        .iter()
        .find(|&&(r, at, _)| r == room && at.is_none())
        .map(|&(.., why)| why)
}

/// The row `rows` has for `cell`, if any.
fn row(rows: &[(&str, Mood, bool, [f64; 4])], (room, mood, quiet): Cell) -> Option<[f64; 4]> {
    rows.iter()
        .find(|&&(r, m, q, _)| r == room && m == mood && q == quiet)
        .map(|&(.., row)| row)
}

/// `cell`'s baseline, its σ carried to the band's visits.
fn baseline(cell: Cell) -> Stats {
    let [share, share_sd, rate, rate_sd] =
        row(&BASELINE, cell).unwrap_or_else(|| panic!("no baseline for {cell:?}"));
    Stats {
        share,
        share_sd: share_sd * shorter_visits(),
        rate,
        rate_sd: rate_sd * shorter_visits(),
    }
}

/// What `cell` is aimed at, with σ: its [`TUNED`] row, else its band's
/// middle and its mood's cap, with the baseline's σ (the rate's scaled
/// to the cap).
fn aim(cell: Cell) -> Stats {
    if let Some([share, share_sd, rate, rate_sd]) = row(&TUNED, cell) {
        return Stats {
            share,
            share_sd,
            rate,
            rate_sd,
        };
    }
    let base = baseline(cell);
    let cap = Band::of(cell.0, cell.1).cap;
    Stats {
        share: Band::of(cell.0, cell.1).middle(),
        share_sd: base.share_sd,
        rate: cap,
        rate_sd: base.rate_sd * cap / base.rate,
    }
}

/// σ of a mean of `n` band visits, from σ of a [`SET`]'s.
fn sd_of(set_sd: f64, n: u64) -> f64 {
    set_sd * (SET / n as f64).sqrt()
}

/// The fewest visits for `3σ ≤ room`, from σ of a [`SET`]'s mean (`None`:
/// no number of visits, as `room` isn't positive).
fn visits_for(set_sd: f64, room: f64) -> Option<u64> {
    (room > 0.0).then(|| (SET * (SIGMAS * set_sd / room).powi(2)).ceil() as u64)
}

/// "N" or "never".
fn n_or_never(n: Option<u64>) -> String {
    n.map_or("never".to_owned(), |n| n.to_string())
}

/// What a cell's visits came to: moving and time in sight after the
/// warm-up (ms), and set-offs then.
#[derive(Clone, Copy, Default)]
struct Reading {
    moving: u64,
    sight: u64,
    set_offs: usize,
}

impl Reading {
    /// The visits' pooled reading.
    fn of(visits: &[Visit]) -> Reading {
        visits.iter().fold(Reading::default(), |r, v| Reading {
            moving: r.moving + v.moving(),
            sight: r.sight + v.sight[1],
            set_offs: r.set_offs + v.set_off_count(),
        })
    }

    fn add(self, other: Reading) -> Reading {
        Reading {
            moving: self.moving + other.moving,
            sight: self.sight + other.sight,
            set_offs: self.set_offs + other.set_offs,
        }
    }

    /// Moving, % of the time in sight.
    fn share(self) -> f64 {
        100.0 * self.moving as f64 / self.sight.max(1) as f64
    }

    /// Set-offs a minute in sight.
    fn rate(self) -> f64 {
        self.set_offs as f64 / (self.sight as f64 / 60_000.0).max(1e-9)
    }
}

/// Her `n` fed afternoons in `room`, in `mood`, in line art, from seed 0.
fn read(room: &Room, mood: Mood, n: u64) -> Reading {
    let visits: Vec<Visit> = (0..n)
        .map(|seed| afternoon(room, seed, true, mood, BAND_MINUTES).0)
        .collect();
    Reading::of(&visits)
}

/// A cell's band test: `room` (unfed as built; fed here) in `mood`,
/// in each of `quiets` (quiet, and at its cadence), each inside its
/// bounds and under its cap. Prints each condition's line; fails if any
/// fails.
fn band_cell(room: Room, mood: Mood, quiets: &[bool]) {
    let room = at_afternoon(room);
    let n = census_band_seeds(GATE_SEEDS);
    let band = Band::of(room.name, mood);
    let mut lines = Vec::new();
    let mut failed = false;
    for &quiet in quiets {
        let cell = (room.name, mood, quiet);
        let conditioned = with_chat(&room, quiet);
        let aimed = aim(cell);
        let reading = read(&conditioned, mood, n);
        let margin = SIGMAS * sd_of(aimed.share_sd, n);
        // A cell short of the band is held to the band itself.
        let (lo, hi) = match short(cell) {
            Some(_) => (band.floor, band.ceiling),
            None => (
                band.floor.min(aimed.share - margin),
                band.ceiling.max(aimed.share + margin),
            ),
        };
        // The mood's cap, or 3σ over the aimed rate if that's higher.
        let rate_margin = SIGMAS * sd_of(aimed.rate_sd, n);
        let cap = band.cap.max(aimed.rate + rate_margin);
        let (share, rate) = (reading.share(), reading.rate());
        let share_ok = (lo..=hi).contains(&share);
        let rate_ok = rate <= cap;
        failed |= !(share_ok && rate_ok);
        let mark = |ok: bool| if ok { "ok" } else { "FAIL" };
        lines.push(format!(
            "{} {mood:?} {} N={n}: moving {share:.1}% in {lo:.1}–{hi:.1} {} (band {:.0}–{:.0}, aim {:.1}, 3σ {margin:.1}; the band itself from N={}) | set-offs {rate:.2}/min ≤ {cap:.2} {} (cap {:.2}, aim {:.2}, 3σ {rate_margin:.2}; the cap itself from N={}) | {:.1} min in sight",
            room.name,
            chat_name(&conditioned),
            mark(share_ok),
            band.floor,
            band.ceiling,
            aimed.share,
            n_or_never(visits_for(aimed.share_sd, band.room(aimed.share))),
            mark(rate_ok),
            band.cap,
            aimed.rate,
            n_or_never(visits_for(aimed.rate_sd, band.cap - aimed.rate)),
            reading.sight as f64 / 60_000.0,
        ));
        if let Some(why) = short(cell) {
            lines.push(format!("  short of the band at step 8c: {why}"));
        }
    }
    eprintln!("{}", lines.join("\n"));
    assert!(!failed, "outside the stillness band (the lines above)");
}

/// A room's aimed spread (industrious ÷ lazy, both chats pooled) and σ
/// of it at `n` visits a condition (the delta method, from each mood's
/// pooled σ).
fn aimed_spread(room: &'static str, n: u64) -> (f64, f64, f64, f64) {
    let pooled = |mood: Mood| {
        let (mut share, mut var) = (0.0, 0.0);
        for quiet in [true, false] {
            let aimed = aim((room, mood, quiet));
            share += aimed.share / 2.0;
            var += (sd_of(aimed.share_sd, n) / 2.0).powi(2);
        }
        (share, var.sqrt())
    };
    let (lazy, lazy_sd) = pooled(Mood::Lazy);
    let (busy, busy_sd) = pooled(Mood::Industrious);
    let ratio = busy / lazy;
    let sd = ratio * ((busy_sd / busy).powi(2) + (lazy_sd / lazy).powi(2)).sqrt();
    (ratio, sd, busy, lazy)
}

/// A room's spread between moods (B5): industrious moves at least
/// [`SPREAD`]× as much as lazy, both chat conditions pooled, at the same
/// seeds. Its floor stands 3σ below the aims' ratio too, so at the
/// gate's few seeds it asks nothing (a floor under zero) for the cost
/// of two band tests: it stays out of the gate, and is [`SPREAD`] from
/// the N its line prints.
fn band_spread(room: Room) {
    let room = at_afternoon(room);
    let n = census_band_seeds(GATE_SEEDS);
    let pooled = |mood: Mood| {
        [true, false]
            .into_iter()
            .fold(Reading::default(), |r, quiet| {
                r.add(read(&with_chat(&room, quiet), mood, n))
            })
            .share()
    };
    let (lazy, busy) = (pooled(Mood::Lazy), pooled(Mood::Industrious));
    let (aimed, sd, busy_aim, lazy_aim) = aimed_spread(room.name, n);
    // A room short of the spread is held to it itself.
    let floor = match short_spread(room.name) {
        Some(_) => SPREAD,
        None => SPREAD.min(aimed - SIGMAS * sd),
    };
    // σ of the ratio goes as 1/√n, like a mean's.
    let full = visits_for(sd * (n as f64 / SET).sqrt(), aimed - SPREAD);
    let ratio = busy / lazy.max(1e-9);
    eprintln!(
        "{} spread N={n} a mood, both chats: industrious {busy:.1}% ÷ lazy {lazy:.1}% = {ratio:.2} ≥ {floor:.2} {} (aims {busy_aim:.1} ÷ {lazy_aim:.1} = {aimed:.2}, 3σ {:.2}; {SPREAD} itself from N={})",
        room.name,
        if ratio >= floor { "ok" } else { "FAIL" },
        SIGMAS * sd,
        n_or_never(full),
    );
    if let Some(why) = short_spread(room.name) {
        eprintln!("  short of the spread at step 8c: {why}");
    }
    assert!(ratio >= floor, "her moods don't spread (the line above)");
}

/// A band visit outlasts the warm-up and twice her longest still act at
/// the afternoon (B5): an act lengthened past it makes [`BAND_MINUTES`]
/// grow with it.
#[test]
fn band_visits_outlast_her_longest_still_act_twice() {
    let longest = longest_still_ms(Some(Slot::Afternoon));
    assert!(
        BAND_MINUTES * 60_000 >= WARM_MS + 2 * longest,
        "{BAND_MINUTES} minutes is under the warm-up's {} s and twice {} s",
        WARM_MS / 1000,
        longest / 1000
    );
}

/// The band's rule and tables are whole, and its aims meet the band:
/// [`Band::of`] is the user's round-3 table (floors, ceilings and caps
/// rising from lazy through ordinary and dreamy, alike, to industrious);
/// the census rooms are named as [`ROOMS`] has them; [`BASELINE`] has
/// every cell once; [`TUNED`] is empty or has every cell once; every
/// aim (tuned, or the untuned middle) is strictly inside its mood's
/// band, a tuned one strictly under its cap (an untuned one aims at the
/// cap itself), and each room's aims spread more than [`SPREAD`]. So no
/// aim can widen a cell's bounds past the band at full strength: the
/// widening is only ever for σ.
#[test]
fn band_cells_are_whole_and_tuned_inside_the_band() {
    let edges = |room: &str, mood: Mood| {
        let band = Band::of(room, mood);
        (band.floor, band.ceiling, band.cap)
    };
    for room in ROOMS {
        let stage = room == "stage";
        assert_eq!(
            Mood::ALL.map(|mood| edges(room, mood)),
            Mood::ALL.map(|mood| match mood {
                Mood::Lazy => (if stage { 4.0 } else { 5.0 }, 17.0, 1.5),
                Mood::Ordinary | Mood::Dreamy => (if stage { 6.0 } else { 8.0 }, 26.0, 2.25),
                Mood::Industrious => (if stage { 9.0 } else { 12.0 }, 31.0, 3.0),
            }),
            "{room}: the user's round-3 band, by mood, the bare stage's floors lower (step 8c)"
        );
        let rising = [Mood::Lazy, Mood::Ordinary, Mood::Industrious].map(|mood| edges(room, mood));
        for pair in rising.windows(2) {
            let ((f0, c0, k0), (f1, c1, k1)) = (pair[0], pair[1]);
            assert!(
                f0 < f1 && c0 < c1 && k0 < k1,
                "{room}: the band rises with her mood: {pair:?}"
            );
        }
        assert_eq!(
            edges(room, Mood::Ordinary),
            edges(room, Mood::Dreamy),
            "{room}: ordinary and dreamy share a band"
        );
    }
    for (room, mood) in ROOMS
        .into_iter()
        .flat_map(|room| Mood::ALL.map(|mood| (room, mood)))
    {
        let band = Band::of(room, mood);
        assert!(
            0.0 < band.floor && band.floor < band.ceiling && band.cap > 0.0,
            "{mood:?}: floor {}, ceiling {}, cap {}",
            band.floor,
            band.ceiling,
            band.cap
        );
    }
    assert_eq!(
        [stage_room(), furnished_room(), resident_room()].map(|room| room.name),
        ROOMS,
        "the census rooms' names"
    );
    let once = |rows: &[(&str, Mood, bool, [f64; 4])], name: &str| {
        assert_eq!(rows.len(), 24, "{name}: a row a cell");
        for cell in cells() {
            let n = rows.iter().filter(|r| row(&[**r], cell).is_some()).count();
            assert_eq!(n, 1, "{name}: {cell:?} {n} times");
        }
    };
    once(&BASELINE, "BASELINE");
    if !TUNED.is_empty() {
        once(&TUNED, "TUNED");
    }
    // What's short is exactly what the user shipped short (step 8c), and
    // its tables say so. The literal is a deliberate tripwire: the list
    // is the user's signed-off one, so growing it is a change to ask
    // them about, not a number to refresh.
    assert_eq!(
        SHORT.map(|(room, at, _)| (room, at)),
        [("stage", Some((Mood::Industrious, false))), ("stage", None)],
        "SHORT"
    );
    for (room, at, _) in SHORT {
        match at {
            Some((mood, quiet)) => {
                let aimed = aim((room, mood, quiet));
                assert!(
                    !Band::of(room, mood).holds(aimed.share),
                    "{room} {mood:?} {quiet}: inside its band now (take it off SHORT)"
                );
            }
            None => assert!(
                aimed_spread(room, GATE_SEEDS).0 <= SPREAD,
                "{room}: spread now (take it off SHORT)"
            ),
        }
    }
    for cell in cells().filter(|&cell| short(cell).is_none()) {
        let (aimed, band) = (aim(cell), Band::of(cell.0, cell.1));
        assert!(
            band.holds(aimed.share),
            "{cell:?}: aimed at {:.1}%, outside {:.0}–{:.0}",
            aimed.share,
            band.floor,
            band.ceiling
        );
        let tuned = row(&TUNED, cell).is_some();
        assert!(
            aimed.rate < band.cap || !tuned && aimed.rate == band.cap,
            "{cell:?}: aimed at {:.2} set-offs a minute, not under its cap {:.2}",
            aimed.rate,
            band.cap
        );
    }
    for room in ROOMS
        .into_iter()
        .filter(|&room| short_spread(room).is_none())
    {
        let (ratio, ..) = aimed_spread(room, GATE_SEEDS);
        assert!(
            ratio > SPREAD,
            "{room}: aims spread {ratio:.2}, not over {SPREAD}"
        );
    }
}

/// One band test per room and mood.
macro_rules! band {
    ($($name:ident: $room:ident $mood:ident;)*) => {$(
        #[test]
        fn $name() {
            band_cell($room(), Mood::$mood, &[true, false]);
        }
    )*};
}

band! {
    band_stage_lazy: stage_room Lazy;
    band_stage_ordinary: stage_room Ordinary;
    band_stage_dreamy: stage_room Dreamy;
    band_home_lazy: furnished_room Lazy;
    band_home_ordinary: furnished_room Ordinary;
    band_home_dreamy: furnished_room Dreamy;
    band_home_industrious: furnished_room Industrious;
    band_resident_lazy: resident_room Lazy;
    band_resident_ordinary: resident_room Ordinary;
    band_resident_dreamy: resident_room Dreamy;
    band_resident_industrious: resident_room Industrious;
}

/// The stage's industrious afternoon, quiet: inside the band, in the
/// gate (its chat cell is short: [`band_stage_industrious_chat`]).
#[test]
fn band_stage_industrious_quiet() {
    band_cell(stage_room(), Mood::Industrious, &[true]);
}

/// Short of the band (see [`SHORT`]): at full strength, the stage's
/// industrious afternoon with chat is under its floor.
#[test]
#[ignore = "5c step 8c, shipped short: 8.4% with chat against the stage's industrious floor of 9 at full strength (SHORT)"]
fn band_stage_industrious_chat() {
    band_cell(stage_room(), Mood::Industrious, &[false]);
}

#[test]
#[ignore = "5c step 8c, shipped short: industrious ÷ lazy 1.39 against 1.6 at full strength (SHORT)"]
fn band_spread_stage() {
    band_spread(stage_room());
}

#[test]
#[ignore = "5c: asks nothing at the gate's seeds (a floor under zero); run at full strength"]
fn band_spread_home() {
    band_spread(furnished_room());
}

#[test]
#[ignore = "5c: asks nothing at the gate's seeds (a floor under zero); run at full strength"]
fn band_spread_resident() {
    band_spread(resident_room());
}
