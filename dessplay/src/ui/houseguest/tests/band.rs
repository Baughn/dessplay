//! The stillness band (phase 5c, D2 as amended by B5 and M3): how much
//! she moves on a fed afternoon ([`afternoon`]), per census room and
//! mood, quiet and at the room's chat cadence, each condition judged on
//! its own. Moving is the census's: moving in sight after the warm-up
//! ([`WARM_MS`]) as a share of her time in sight then, pooled over the
//! cell's visits (Σ moving ÷ Σ in sight), and her set-offs a minute in
//! sight.
//!
//! **Line art only.** The houseguest's tests otherwise loop both drawing
//! modes; these don't, by design: the band is pinned in line art (the
//! user's client draws it), and the home and resident run the same in
//! either mode, seed for seed (phase5c/baseline.md: neither has text she
//! can reach), so a second mode would double the gate's cost to measure
//! the stage's ASCII alone, which the user's client doesn't draw.
//!
//! **Two strengths, one rule** (B5): every threshold stands at least 3σ
//! from the cell's aim, σ being that of the mean of the visits run. With
//! `CENSUS_BAND_SEEDS` at full strength (the most any line asks for:
//! each band line prints the N at which its bounds are the band itself,
//! and its cap M3's own; each spread line the N at which its floor is
//! 1.6) the thresholds are the band's. Run at step boundaries:
//!
//! ```text
//! cargo nextest run -p dessplay --run-ignored only -E 'test(/houseguest::tests::band::/)'
//! CENSUS_BAND_SEEDS=<n> cargo nextest run -p dessplay --release --profile band \
//!   --run-ignored only -E 'test(/houseguest::tests::band::/)' --success-output final
//! ```
//!
//! (`--no-capture` would run them one at a time; `--profile band` gives
//! them minutes each.)
//!
//! **What the gate's seeds can see.** One visit's reading varies widely
//! (a 9-minute stage visit reads anywhere from 2% to 77%), so at the
//! gate's [`GATE_SEEDS`] 3σ is 15–31 points. No share floor is above 11
//! and most are below zero, so a "too still" regression all but never
//! fails there; the stage's share bounds reach from about −15–2 to
//! 42–55, so its share check can't fail for any plausible reading. The
//! gate checks the set-off cap (with ×1.2–1.6 slack over M3's) and a
//! gross excess of moving in the home and resident, no more. Full
//! strength is the real check: passing at the gate is not evidence the
//! band is met. B5's fallback (the floors pooled per mood across rooms)
//! would gain √3 on the floors and nothing on the ceilings, so it isn't
//! built: whether to spend more of the gate on the band is the user's
//! call (phase 5c, step 4's hand-off).
//!
//! **Not here yet:** D2's printed TV-only home, and a home or resident
//! with text at her floors' heights (baseline.md: only the stage has
//! text she can reach, so only it shows line-art terrain and M5's
//! text-locality levers). Deferred to the band's re-read on a windowed
//! home (phase 5c step 11, minor 9).
//!
//! Ignored until the tuning meets the band: step 8 stopped short of it,
//! on M3's set-off cap, which waits on the user (phase5c/baseline.md,
//! "Step 8: tuning stopped").

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
/// luck. Today that's a day's sleep, 3 minutes at most: 3 + 2 × 3.
/// [`band_visits_outlast_her_longest_still_act_twice`] holds it to that.
const BAND_MINUTES: u64 = 9;

/// The seeds a cell runs at the gate (`CENSUS_BAND_SEEDS` overrides):
/// what the stage, the costliest room, affords in about 3 s a test (a
/// 9-minute line-art visit costs about 0.7 s there under the gate's
/// opt-level 2, 0.6 s in the home and 0.4 s in the resident; a test runs
/// two conditions). Those are a test run alone: in the whole gate's
/// parallel run they took 3.1–6.0 s, and un-ignoring the twelve added
/// about 1 s to its 29 s wall time (2026-10-06, 32 threads; the long
/// pole is elsewhere).
const GATE_SEEDS: u64 = 2;

/// The margin every threshold keeps from the aim, in σ of the cell's
/// mean (B5).
const SIGMAS: f64 = 3.0;

/// The visits a σ in [`BASELINE`] or [`TUNED`] is of the mean of.
const SET: f64 = 4.0;

/// The minutes [`BASELINE`]'s visits ran: [`BAND_MINUTES`], since step
/// 5's re-baseline (`CENSUS_MINUTES=9 CENSUS_MODES=line`), so
/// [`shorter_visits`] is 1.
const BASELINE_MINUTES: u64 = 9;

/// The industrious ÷ lazy spread each room's moving keeps (B5).
const SPREAD: f64 = 1.6;

/// σ of a band visit's reading over a baseline visit's: a share or rate
/// read over less time in sight varies more, about as 1/√time (her acts
/// as the independent draws). It carries [`BASELINE`]'s σ to the band's
/// shorter visits (approximately: 1 once the baseline is read at
/// [`BAND_MINUTES`]). Never applied to [`TUNED`]'s, read at the band's
/// own length.
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

/// A mood's band for moving in sight (% of her time in sight), and the
/// user's own target in it (the set-off cap's, M3).
#[derive(Clone, Copy)]
struct Band {
    floor: f64,
    ceiling: f64,
    target: f64,
}

impl Band {
    /// The user's "about 15 lazy, 20–25 ordinary and dreamy, up to 30
    /// industrious", as B5 bounds it: ceilings a point or two over,
    /// floors the target less 4 ("too still").
    fn of(mood: Mood) -> Band {
        match mood {
            Mood::Lazy => Band {
                floor: 11.0,
                ceiling: 17.0,
                target: 15.0,
            },
            Mood::Ordinary | Mood::Dreamy => Band {
                floor: 16.0,
                ceiling: 26.0,
                target: 22.5,
            },
            Mood::Industrious => Band {
                floor: 20.0,
                ceiling: 31.0,
                target: 30.0,
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

/// The baseline (phase5c/baseline.md, "After the shorter watch"),
/// [`BASELINE_MINUTES`] long: moving %, σ of a 4-visit mean, set-offs a
/// minute, its σ. Every row is line art; the home's and resident's stand
/// for ASCII too on step 3's finding that those rooms don't depend on
/// drawing mode (no text in her reach there), not re-measured since.
// Measured numbers: a rate of 3.14 a minute is not π.
#[allow(clippy::approx_constant)]
const BASELINE: [(&str, Mood, bool, [f64; 4]); 24] = [
    ("stage", Mood::Ordinary, true, [32.5, 6.98, 4.96, 0.67]),
    ("stage", Mood::Ordinary, false, [30.7, 7.26, 5.32, 0.57]),
    ("stage", Mood::Lazy, true, [21.8, 6.55, 4.19, 0.30]),
    ("stage", Mood::Lazy, false, [24.6, 6.89, 4.74, 0.41]),
    ("stage", Mood::Industrious, true, [34.9, 6.93, 5.11, 0.51]),
    ("stage", Mood::Industrious, false, [35.3, 5.60, 5.66, 0.56]),
    ("stage", Mood::Dreamy, true, [27.8, 5.25, 4.86, 0.43]),
    ("stage", Mood::Dreamy, false, [29.5, 6.27, 5.24, 0.66]),
    ("home", Mood::Ordinary, true, [48.4, 5.16, 2.33, 0.20]),
    ("home", Mood::Ordinary, false, [48.6, 5.68, 2.67, 0.20]),
    ("home", Mood::Lazy, true, [35.8, 4.65, 1.51, 0.18]),
    ("home", Mood::Lazy, false, [39.0, 3.66, 2.06, 0.16]),
    ("home", Mood::Industrious, true, [53.4, 6.26, 2.72, 0.26]),
    ("home", Mood::Industrious, false, [54.0, 4.42, 3.08, 0.30]),
    ("home", Mood::Dreamy, true, [46.1, 4.11, 2.27, 0.17]),
    ("home", Mood::Dreamy, false, [47.7, 4.35, 2.61, 0.21]),
    ("resident", Mood::Ordinary, true, [38.8, 4.97, 2.79, 0.20]),
    ("resident", Mood::Ordinary, false, [37.8, 5.89, 3.14, 0.24]),
    ("resident", Mood::Lazy, true, [32.3, 3.71, 2.31, 0.12]),
    ("resident", Mood::Lazy, false, [35.0, 5.48, 2.78, 0.17]),
    (
        "resident",
        Mood::Industrious,
        true,
        [43.9, 5.43, 2.92, 0.16],
    ),
    (
        "resident",
        Mood::Industrious,
        false,
        [41.2, 3.51, 3.20, 0.21],
    ),
    ("resident", Mood::Dreamy, true, [36.6, 4.65, 2.70, 0.22]),
    ("resident", Mood::Dreamy, false, [35.1, 4.00, 3.01, 0.20]),
];

/// The tuned cells, which the tuning fills from one tuned census read at
/// [`BAND_MINUTES`] (`CENSUS_MINUTES=9 CENSUS_MODES=line`, line art):
/// moving %, σ of a 4-visit mean, set-offs a minute, its σ, by room,
/// mood and quiet. Empty, or every cell once
/// ([`band_cells_are_whole_and_tuned_inside_the_band`] holds it, and that
/// each aim is inside its band, under M3's cap and spread 1.6 a room):
/// while empty, a cell aims at its band's middle and at M3's cap, with
/// the baseline's σ.
const TUNED: [(&str, Mood, bool, [f64; 4]); 0] = [];

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

/// M3's set-off cap for `cell`: the baseline's rate as it would fall if
/// her moving fell to the user's target by her moving less often, so a
/// lever that only shortens walks fails it.
fn m3_cap(cell: Cell) -> f64 {
    let base = baseline(cell);
    base.rate * Band::of(cell.1).target / base.share
}

/// What `cell` is aimed at, with σ: its [`TUNED`] row, else its band's
/// middle and M3's cap, with the baseline's σ (the rate's scaled to the
/// cap).
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
    let cap = m3_cap(cell);
    Stats {
        share: Band::of(cell.1).middle(),
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
/// quiet and at its cadence, each inside its bounds and under its cap.
/// Prints each condition's line; fails if any fails.
fn band_cell(room: Room, mood: Mood) {
    let room = at_afternoon(room);
    let n = census_band_seeds(GATE_SEEDS);
    let band = Band::of(mood);
    let mut lines = Vec::new();
    let mut failed = false;
    for quiet in [true, false] {
        let cell = (room.name, mood, quiet);
        let conditioned = with_chat(&room, quiet);
        let (base, aimed, m3) = (baseline(cell), aim(cell), m3_cap(cell));
        let reading = read(&conditioned, mood, n);
        let margin = SIGMAS * sd_of(aimed.share_sd, n);
        let (lo, hi) = (
            band.floor.min(aimed.share - margin),
            band.ceiling.max(aimed.share + margin),
        );
        // M3's cap, or 3σ over the aimed rate if that's higher (the
        // untuned aim is M3's cap itself, so it's always 3σ over).
        let rate_margin = SIGMAS * sd_of(aimed.rate_sd, n);
        let cap = m3.max(aimed.rate + rate_margin);
        let (share, rate) = (reading.share(), reading.rate());
        let share_ok = (lo..=hi).contains(&share);
        let rate_ok = rate <= cap;
        failed |= !(share_ok && rate_ok);
        let mark = |ok: bool| if ok { "ok" } else { "FAIL" };
        lines.push(format!(
            "{} {mood:?} {} N={n}: moving {share:.1}% in {lo:.1}–{hi:.1} {} (band {:.0}–{:.0}, aim {:.1}, 3σ {margin:.1}; the band itself from N={}) | set-offs {rate:.2}/min ≤ {cap:.2} {} (M3 {m3:.2} from the baseline's {:.2} at {:.1}%, aim {:.2}, 3σ {rate_margin:.2}; M3's own from N={}) | {:.1} min in sight",
            room.name,
            chat_name(&conditioned),
            mark(share_ok),
            band.floor,
            band.ceiling,
            aimed.share,
            n_or_never(visits_for(aimed.share_sd, band.room(aimed.share))),
            mark(rate_ok),
            base.rate,
            base.share,
            aimed.rate,
            n_or_never(visits_for(aimed.rate_sd, m3 - aimed.rate)),
            reading.sight as f64 / 60_000.0,
        ));
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
/// gate's few seeds it asks next to nothing (a floor under zero, at the
/// baseline's σ) for the cost of two band tests: it stays out of the
/// gate, and is [`SPREAD`] from the N its line prints.
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
    let floor = SPREAD.min(aimed - SIGMAS * sd);
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

/// The band's tables are whole, and its aims meet the band: the census
/// rooms are named as [`ROOMS`] has them; [`BASELINE`] has every cell
/// once; [`TUNED`] is empty or has every cell once, each aim strictly
/// inside its band and under M3's cap, and each room's aims spread more
/// than [`SPREAD`]. So no aim can widen a cell's bounds past the band at
/// full strength: the widening is only ever for σ.
#[test]
fn band_cells_are_whole_and_tuned_inside_the_band() {
    assert_eq!(
        [stage_room(), furnished_room(), resident_room()].map(|room| room.name),
        ROOMS,
        "the census rooms' names"
    );
    let once = |rows: &[(&str, Mood, bool, [f64; 4])], name: &str| {
        assert_eq!(rows.len(), 24, "{name}: a row a cell");
        for cell in cells() {
            let (room, mood, quiet) = cell;
            let n = rows
                .iter()
                .filter(|&&(r, m, q, _)| r == room && m == mood && q == quiet)
                .count();
            assert_eq!(n, 1, "{name}: {cell:?} {n} times");
        }
    };
    once(&BASELINE, "BASELINE");
    if TUNED.is_empty() {
        return;
    }
    once(&TUNED, "TUNED");
    for cell in cells() {
        let (aimed, band, m3) = (aim(cell), Band::of(cell.1), m3_cap(cell));
        assert!(
            band.holds(aimed.share),
            "{cell:?}: aimed at {:.1}%, outside {:.0}–{:.0}",
            aimed.share,
            band.floor,
            band.ceiling
        );
        assert!(
            aimed.rate < m3,
            "{cell:?}: aimed at {:.2} set-offs a minute, not under M3's {m3:.2}",
            aimed.rate
        );
    }
    for room in ROOMS {
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
        #[ignore = "5c: ignored until the user decides M3's set-off cap (phase5c/baseline.md, Step 8: tuning stopped)"]
        fn $name() {
            band_cell($room(), Mood::$mood);
        }
    )*};
}

band! {
    band_stage_lazy: stage_room Lazy;
    band_stage_ordinary: stage_room Ordinary;
    band_stage_dreamy: stage_room Dreamy;
    band_stage_industrious: stage_room Industrious;
    band_home_lazy: furnished_room Lazy;
    band_home_ordinary: furnished_room Ordinary;
    band_home_dreamy: furnished_room Dreamy;
    band_home_industrious: furnished_room Industrious;
    band_resident_lazy: resident_room Lazy;
    band_resident_ordinary: resident_room Ordinary;
    band_resident_dreamy: resident_room Dreamy;
    band_resident_industrious: resident_room Industrious;
}

#[test]
#[ignore = "5c: asks nothing at the gate's seeds (a floor under zero); run at full strength"]
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
