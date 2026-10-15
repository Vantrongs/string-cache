// Copyright 2014 The Servo Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use hashbrown::hash_table::{Entry as TableEntry, HashTable};
use parking_lot::Mutex;
use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::Ordering::SeqCst;

/// Each shard has its own lock, so that threads interning different strings
/// seldom wait for one another.
const NB_SHARDS: usize = 1 << 8;

/// A shard's table is shrunk once it is filled to less than a quarter, so that
/// it takes memory in the atoms alive rather than the most that ever were.
const MIN_CAPACITY_TO_SHRINK: usize = 64;

pub(crate) struct Set {
    /// Keys the hash that places a string in a shard and in the shard's table.
    /// It is random in each process, so strings cannot be chosen in advance to
    /// share a place and make every insertion step past all of them.
    hasher: RandomState,
    /// Entries are leaked boxes, freed by `remove` once out of their table.
    shards: Box<[Mutex<HashTable<NonNull<Entry>>>]>,
}

pub(crate) struct Entry {
    pub(crate) string: Box<str>,
    pub(crate) hash: u64,
    pub(crate) ref_count: AtomicIsize,
    /// `string` hashed with the set's `hasher`.
    set_hash: u64,
}

// SAFETY: the tables hold pointers to entries only while their shard's lock is
// held, and an entry's fields are immutable except for its atomic count.
unsafe impl Send for Set {}
unsafe impl Sync for Set {}

pub(crate) fn dynamic_set() -> &'static Set {
    // NOTE: Using const initialization for shards breaks the small-stack test.
    static DYNAMIC_SET: OnceLock<Set> = OnceLock::new();

    DYNAMIC_SET.get_or_init(|| Set {
        hasher: RandomState::new(),
        shards: (0..NB_SHARDS)
            .map(|_| Mutex::new(HashTable::new()))
            .collect(),
    })
}

fn set_hash_of(ptr: &NonNull<Entry>) -> u64 {
    // SAFETY: a table holds only live entries; `remove` takes an entry out of
    // its table, under the shard's lock, before it frees it.
    unsafe { ptr.as_ref() }.set_hash
}

impl Set {
    fn shard(&self, set_hash: u64) -> &Mutex<HashTable<NonNull<Entry>>> {
        // A table places entries by the low bits of the hash and tells them
        // apart by the top seven, so the shard takes bits from the middle.
        &self.shards[(set_hash >> 32) as usize % NB_SHARDS]
    }

    pub(crate) fn insert(&self, string: Cow<str>, hash: u64) -> NonNull<Entry> {
        let set_hash = self.hasher.hash_one(&*string);
        let mut table = self.shard(set_hash).lock();
        // Atoms of static sets with different keys hash a string differently,
        // so each set has its own entry for it, as `get_hash` returns `hash`.
        let same_atom = |ptr: &NonNull<Entry>| {
            // SAFETY: as in `set_hash_of`.
            let entry = unsafe { ptr.as_ref() };
            entry.hash == hash && *entry.string == *string
        };
        let vacant = match table.entry(set_hash, same_atom, set_hash_of) {
            TableEntry::Occupied(occupied) => {
                let ptr = *occupied.get();
                // SAFETY: as in `set_hash_of`.
                let entry = unsafe { ptr.as_ref() };
                let old_count = entry.ref_count.fetch_add(1, SeqCst);
                if old_count > 0 {
                    if old_count == isize::MAX {
                        std::process::abort();
                    }
                    return ptr;
                }
                // The entry's last atom is being dropped, and its `remove` waits
                // for this lock to free it. (Naive attempts to defend against this,
                // for example having `remove` check to see whether the reference
                // count is indeed zero, don't work due to ABA.) So the entry leaves
                // the table, for `remove` to free it, and a new one takes its place.
                entry.ref_count.fetch_sub(1, SeqCst);
                occupied.remove().1
            }
            TableEntry::Vacant(vacant) => vacant,
        };
        let entry = Box::new(Entry {
            string: string.into_owned().into_boxed_str(),
            hash,
            ref_count: AtomicIsize::new(1),
            set_hash,
        });
        let ptr = NonNull::from(Box::leak(entry));
        vacant.insert(ptr);
        ptr
    }

    pub(crate) fn remove(&self, ptr: *mut Entry) {
        // SAFETY: `ptr` is the entry of an atom whose drop took its count to
        // zero, and only this call frees it.
        let set_hash = unsafe { &*ptr }.set_hash;
        let mut table = self.shard(set_hash).lock();
        debug_assert!(unsafe { &*ptr }.ref_count.load(SeqCst) == 0);
        // `insert` takes the entry out of the table itself when it finds it dead.
        if let Ok(occupied) = table.find_entry(set_hash, |p| p.as_ptr() == ptr) {
            occupied.remove();
            let len = table.len();
            if table.capacity() >= MIN_CAPACITY_TO_SHRINK && len * 4 < table.capacity() {
                table.shrink_to(len * 2, set_hash_of);
            }
        }
        drop(table);
        // SAFETY: the entry is out of the table, so no other thread can reach it
        // (`Atom` methods hold no reference to it past the dropped atom). The
        // memory is freed after releasing the lock.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[cfg(feature = "malloc_size_of")]
pub fn malloc_size_of_dynamic_set(ops: &mut malloc_size_of::MallocSizeOfOps) -> usize {
    let mut sum = 0;
    for shard in &dynamic_set().shards {
        let table = shard.lock();
        sum += table.allocation_size();
        for ptr in table.iter() {
            // We would use `<Box<Entry> as malloc_size_of::MallocSizeOf>` here,
            // but we don’t have `&Box<Entry>` only `NonNull<Entry>`.
            // SAFETY: `ptr` is a valid heap-allocated pointer
            sum += unsafe { ops.malloc_size_of::<Entry>(ptr.as_ptr()) };
            // SAFETY: as in `set_hash_of`.
            let entry = unsafe { ptr.as_ref() };
            sum += <Box<str> as malloc_size_of::MallocSizeOf>::size_of(&entry.string, ops);
        }
    }
    sum
}
