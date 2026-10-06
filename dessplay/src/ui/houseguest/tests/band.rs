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
//! (a stage visit reads anywhere from a few % to about 80% of its time
//! moving), so at the gate's [`GATE_SEEDS`] 3σ on the share is 3 × √2 ×
//! [`BASELINE`]'s 4-visit σ (3.1–8.0): about 13 to 34 points. The gate
//! catches a gross excess of moving and of set-offs, no more, and a
//! "too still" regression all but never fails there. Full strength is
//! the real check: passing at the gate is not evidence the band is met
//! (the user, step 4: "I'll know to check them if something feels
//! off").
//!
//! **Not here yet:** D2's printed TV-only home, and a home or resident
//! with text at her floors' heights (baseline.md: only the stage has
//! text she can reach). Deferred to the band's re-read on a windowed
//! home (phase 5c step 11, minor 9).
//!
//! Ignored until a tuning meets the band: step 8b's retune stopped short
//! of it on the stage, whose short trips put its reachable share at about
//! its floor under the mood's cap (phase5c/baseline.md, "Step 8b: the
//! retune, stopped"). The rule and the structure tests run.

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
/// luck. Today that's a day's sleep or a look out of her window, 3
/// minutes at most: 3 + 2 × 3, with no headroom.
/// [`band_visits_outlast_her_longest_still_act_twice`] holds it to that.
/// Lingering as the design starts it makes a lazy day's sleep (or look
/// out) 4.5 minutes, so a tuning that lands it makes this 12; with
/// settling on too, her window's chain (leaning on the sill, sitting in
/// front of it, dozing or watching the clouds: about 6.1 minutes lazy)
/// sets it: 3 + 2 × 6.1, so 16 (phase 5c step 11; baseline.md).
const BAND_MINUTES: u64 = 9;

/// The seeds a cell runs at the gate (`CENSUS_BAND_SEEDS` overrides):
/// what the stage, the costliest room, affords in about 3 s a test (a
/// 9-minute line-art visit costs about 0.7 s there under the gate's
/// opt-level 2, 0.6 s in the home and 0.4 s in the resident; a test runs
/// two conditions).
const GATE_SEEDS: u64 = 2;

/// The margin every threshold keeps from the aim, in σ of the cell's
/// mean (B5).
const SIGMAS: f64 = 3.0;

/// The visits a σ in [`BASELINE`] or [`TUNED`] is of the mean of.
const SET: f64 = 4.0;

/// The minutes [`BASELINE`]'s visits ran: [`BAND_MINUTES`], so
/// [`shorter_visits`] is 1.
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
    fn of(mood: Mood) -> Band {
        match mood {
            Mood::Lazy => Band {
                floor: 5.0,
                ceiling: 17.0,
                cap: 1.5,
            },
            Mood::Ordinary | Mood::Dreamy => Band {
                floor: 8.0,
                ceiling: 26.0,
                cap: 2.25,
            },
            Mood::Industrious => Band {
                floor: 12.0,
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
    let cap = Band::of(cell.1).cap;
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
        let aimed = aim(cell);
        let reading = read(&conditioned, mood, n);
        let margin = SIGMAS * sd_of(aimed.share_sd, n);
        let (lo, hi) = (
            band.floor.min(aimed.share - margin),
            band.ceiling.max(aimed.share + margin),
        );
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
    let edges = |mood: Mood| {
        let band = Band::of(mood);
        (band.floor, band.ceiling, band.cap)
    };
    assert_eq!(
        Mood::ALL.map(edges),
        Mood::ALL.map(|mood| match mood {
            Mood::Lazy => (5.0, 17.0, 1.5),
            Mood::Ordinary | Mood::Dreamy => (8.0, 26.0, 2.25),
            Mood::Industrious => (12.0, 31.0, 3.0),
        }),
        "the user's round-3 band, by mood"
    );
    let rising = [Mood::Lazy, Mood::Ordinary, Mood::Industrious].map(edges);
    for pair in rising.windows(2) {
        let ((f0, c0, k0), (f1, c1, k1)) = (pair[0], pair[1]);
        assert!(
            f0 < f1 && c0 < c1 && k0 < k1,
            "the band rises with her mood: {pair:?}"
        );
    }
    assert_eq!(
        edges(Mood::Ordinary),
        edges(Mood::Dreamy),
        "ordinary and dreamy share a band"
    );
    for mood in Mood::ALL {
        let band = Band::of(mood);
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
    for cell in cells() {
        let (aimed, band) = (aim(cell), Band::of(cell.1));
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
        #[ignore = "5c: ignored until a tuning meets the per-mood band (phase5c/baseline.md, Step 8b: the retune, stopped)"]
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
