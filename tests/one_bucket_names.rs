// Strings chosen to share a bucket of a fixed hash are interned in linear time.
//
// The dynamic set used to put a string in one of 4,096 linked lists picked by
// the low 12 bits of `phf_shared::hash(string, key)`'s `g << 32 | f1`, where
// `key` is the static set's, fixed when the set is generated. Each string
// interned into a list of n others stepped past all of them, so n strings found
// offline to share the list took time in n².

use std::time::Instant;
use string_cache::{DefaultAtom, EmptyStaticAtomSet, StaticAtomSet};

/// `n` and 11 hex digits of a counter, for each counter that puts the name in
/// bucket 0 under the key of `EmptyStaticAtomSet`, in order. The file holds the
/// differences between the counters as LEB128 numbers.
fn names() -> Vec<String> {
    let mut names = Vec::new();
    let (mut counter, mut delta, mut shift) = (0u64, 0u64, 0);
    for byte in include_bytes!("one_bucket_names.bin") {
        delta |= u64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            counter += delta;
            names.push(format!("n{counter:011x}"));
            (delta, shift) = (0, 0);
        }
    }
    names
}

/// Seconds to intern the strings of each list and drop their atoms, the least
/// of five runs, the lists in turn so that both see the same load.
fn seconds_to_intern(lists: [&[String]; 2]) -> [f64; 2] {
    let mut least = [f64::INFINITY; 2];
    for _ in 0..5 {
        for (list, least) in lists.iter().zip(&mut least) {
            let start = Instant::now();
            let atoms: Vec<DefaultAtom> = list.iter().map(|s| DefaultAtom::from(&**s)).collect();
            assert!(
                atoms
                    .iter()
                    .zip(*list)
                    .all(|(a, s)| a.is_dynamic() && **a == **s)
            );
            drop(atoms);
            *least = least.min(start.elapsed().as_secs_f64());
        }
    }
    least
}

#[test]
#[cfg_attr(miri, ignore = "times the interning")]
fn strings_sharing_a_bucket_of_the_static_key_are_interned_in_linear_time() {
    let names = names();
    assert_eq!(names.len(), 16_000);
    let key = EmptyStaticAtomSet::get().key;
    assert!(
        names
            .iter()
            .all(|n| phf_shared::hash(&**n, &key).f1 & 0xFFF == 0)
    );
    // Four times the strings take about four times as long in linear time and
    // sixteen times as long in quadratic time.
    let quarter = &names[..names.len() / 4];
    let [all, part] = seconds_to_intern([&names, quarter]);
    assert!(
        all / part < 10.0,
        "{} strings took {:.1} times as long as {}",
        names.len(),
        all / part,
        quarter.len()
    );
    // And about as long as the same number of strings spread over the buckets.
    let ordinary: Vec<String> = (0..names.len()).map(|i| format!("m{i:011x}")).collect();
    let [chosen, ordinary] = seconds_to_intern([&names, &ordinary]);
    assert!(
        chosen / ordinary < 3.0,
        "strings of one bucket took {:.1} times as long as others",
        chosen / ordinary
    );
}
