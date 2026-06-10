//! CvRDT merge laws on independently evolved replicas (one actor each):
//! commutative, associative, idempotent, and consistent with op replay.

#![no_main]

use dessplay_core::CrdtState;
use dessplay_core::test_support::{ScriptStep, run_script};
use libfuzzer_sys::fuzz_target;

fn pin_actor(mut steps: Vec<ScriptStep>, actor: u8) -> Vec<ScriptStep> {
    for step in &mut steps {
        step.actor = actor;
    }
    steps
}

fn merged(a: &CrdtState, b: &CrdtState) -> CrdtState {
    let mut out = a.clone();
    out.merge(b.clone());
    out
}

fuzz_target!(
    |input: (Vec<ScriptStep>, Vec<ScriptStep>, Vec<ScriptStep>)| {
        let (steps_a, steps_b, steps_c) = input;
        let (a, ops_a) = run_script(&pin_actor(steps_a, 1));
        let (b, ops_b) = run_script(&pin_actor(steps_b, 2));
        let (c, _) = run_script(&pin_actor(steps_c, 3));

        assert_eq!(merged(&a, &b), merged(&b, &a), "merge not commutative");
        assert_eq!(
            merged(&merged(&a, &b), &c),
            merged(&a, &merged(&b, &c)),
            "merge not associative"
        );
        assert_eq!(merged(&a, &a), a, "merge not idempotent");

        let mut replayed = CrdtState::new();
        for (_, op) in ops_a.iter().chain(ops_b.iter()) {
            replayed.apply(op.clone());
        }
        assert_eq!(
            replayed.view(),
            merged(&a, &b).view(),
            "merge disagrees with op replay"
        );
    }
);
