#![no_main]

#[macro_use]
extern crate arbitrary;

use libfuzzer_sys::fuzz_target;
use libfuzzer_sys::arbitrary::Arbitrary;

use etagere::*;

#[derive(Copy, Clone, Arbitrary, Debug)]
enum Evt {
    Alloc(i32, i32),
    Dealloc(usize),
}

fuzz_target!(|events: Vec<Evt>| {
    let mut atlas = BucketedAtlasAllocator::new(size2(1000, 1000));
    let mut allocations = Vec::new();

    for evt in &events {
        match *evt {
            Evt::Alloc(w, h) => {
                if let Some(alloc) = atlas.allocate(size2(w, h)) {
                    assert!(alloc.rectangle.size().width >= w);
                    assert!(alloc.rectangle.size().height >= h);
                    allocations.push(alloc.id);
                }
            }
            Evt::Dealloc(idx) => {
                if idx < allocations.len() {
                    atlas.deallocate(allocations[idx]);
                    allocations.swap_remove(idx);
                }
            }
        }
    }

    for id in allocations {
        atlas.deallocate(id);
    }

    assert!(atlas.is_empty());
    assert_eq!(atlas.allocated_space(), 0);
});
