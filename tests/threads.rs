// Threads that intern and drop the same strings at once, and enough distinct
// ones to grow and shrink the set's tables, always get atoms of the right text,
// and one string's live atoms stay equal.

use std::sync::{Arc, Barrier};
use std::thread;
use string_cache::DefaultAtom;

const THREADS: usize = 8;
/// Rounds, strings interned per round from those of every thread, and strings
/// of each thread alone per round; fewer under Miri, which runs far slower.
const SIZES: (usize, usize, usize) = if cfg!(miri) {
    (2, 200, 50)
} else {
    (20, 20_000, 5_000)
};

#[test]
fn threads_interning_and_dropping_strings_agree() {
    let shared: Arc<Vec<String>> =
        Arc::new((0..32).map(|i| format!("shared string {i}")).collect());
    let barrier = Arc::new(Barrier::new(THREADS));
    let threads: Vec<_> = (0..THREADS)
        .map(|t| {
            let shared = Arc::clone(&shared);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for round in 0..SIZES.0 {
                    // Strings of every thread, interned until each was the
                    // last atom of its entry many times.
                    let mut held: Vec<(DefaultAtom, &str)> = Vec::new();
                    for i in 0..SIZES.1 {
                        let string = &shared[(i * 7 + t + round) % shared.len()];
                        let atom = DefaultAtom::from(&**string);
                        assert_eq!(&*atom, string);
                        if i % 5 == 0 {
                            held.push((atom.clone(), string));
                        }
                        if held.len() > 8 {
                            let (old, old_string) = held.remove(0);
                            // While `old` lives, its string has one entry.
                            assert_eq!(DefaultAtom::from(old_string), old);
                        }
                    }
                    // Strings of this thread only, many at once.
                    let own: Vec<String> = (0..SIZES.2)
                        .map(|i| format!("thread {t} round {round} string {i}"))
                        .collect();
                    let atoms: Vec<DefaultAtom> =
                        own.iter().map(|s| DefaultAtom::from(&**s)).collect();
                    for (atom, string) in atoms.iter().zip(&own) {
                        assert_eq!(&**atom, string);
                        assert_eq!(&DefaultAtom::from(&**string), atom);
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
