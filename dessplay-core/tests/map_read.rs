//! Compatibility checks for the vendored Map's context-free read API.
//! These exercise upstream remove semantics too; DessPlay's own maps still
//! use LWW tombstones and never emit Map::rm.

use crdts::{CmRDT, CvRDT, MVReg, Map};
use proptest::prelude::*;

type TestMap = Map<u8, MVReg<u8, u8>, u8>;

fn check_read(map: &TestMap) {
    let before = postcard::to_allocvec(map).unwrap();
    let borrowed: Vec<_> = map.iter_entries().collect();
    let contextual: Vec<_> = map.iter().map(|ctx| ctx.val).collect();
    assert_eq!(borrowed, contextual);
    for ((key, value), (old_key, old_value)) in borrowed.iter().zip(&contextual) {
        assert!(std::ptr::eq(*key, *old_key));
        assert!(std::ptr::eq(*value, *old_value));
    }
    assert!(borrowed.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert_eq!(postcard::to_allocvec(map).unwrap(), before);
}

proptest! {
    #[test]
    fn borrowed_entries_match_contextual_reads_through_replication(
        steps in prop::collection::vec((0u8..3, 0usize..2, 0u8..16, any::<u8>()), 0..100),
    ) {
        let mut maps = [TestMap::new(), TestMap::new()];
        check_read(&maps[0]);
        for (action, destination, key, value) in steps {
            if action == 2 {
                let other = maps[1 - destination].clone();
                maps[destination].merge(other);
            } else {
                let map = &mut maps[destination];
                let op = if action == 0 {
                    let ctx = map.read_ctx().derive_add_ctx(destination as u8);
                    map.update(key, ctx, |reg, ctx| reg.write(value, ctx))
                } else {
                    map.rm(key, map.get(&key).derive_rm_ctx())
                };
                map.apply(op);
            }
            for map in &maps {
                check_read(map);
            }
        }
    }
}
