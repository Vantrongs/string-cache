// Strings chosen to share a bucket of a fixed hash are interned in linear time.
//
// The dynamic set used to put a string in one of 4,096 linked lists picked by
// `phf_shared::hash(string, key).g & 0xFFF`, where `key` is the static set's,
// fixed when the set is generated. Each string interned into a list of n others
// stepped past all of them, so n strings found offline to share the list took
// time in n².

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

/// Seconds to intern the names and drop their atoms, best of three runs.
fn seconds_to_intern(names: &[String]) -> f64 {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let atoms: Vec<DefaultAtom> = names.iter().map(|n| DefaultAtom::from(&**n)).collect();
            assert!(atoms
                .iter()
                .zip(names)
                .all(|(a, n)| a.is_dynamic() && **a == **n));
            drop(atoms);
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn strings_sharing_a_bucket_of_the_static_key_are_interned_in_linear_time() {
    let names = names();
    assert_eq!(names.len(), 16_000);
    let key = EmptyStaticAtomSet::get().key;
    assert!(names
        .iter()
        .all(|n| phf_shared::hash(&**n, &key).g & 0xFFF == 0));
    // Four times the strings take about four times as long in linear time and
    // sixteen times as long in quadratic time.
    let quarter = &names[..names.len() / 4];
    let ratio = seconds_to_intern(&names) / seconds_to_intern(quarter);
    assert!(
        ratio < 8.0,
        "{} strings took {ratio:.1} times as long as {}",
        names.len(),
        quarter.len()
    );
}
