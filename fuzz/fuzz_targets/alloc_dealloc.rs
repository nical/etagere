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
    let mut atlas = BucketedAtlasAllocator::with_options(
        size2(2048, 2048),
        &AllocatorOptions {
            alignment: size2(4, 8),
            vertical_shelves: false,
            num_columns: 2,
        },
    );

    let mut allocations: Vec<Allocation> = Vec::new();

    for evt in &events {
        match *evt {
            Evt::Alloc(w, h) => {
                if let Some(alloc) = atlas.allocate(size2(w, h)) {
                    assert!(alloc.rectangle.size().width >= w);
                    assert!(alloc.rectangle.size().height >= h);

                    for previous in &allocations {
                        assert!(!alloc.rectangle.intersects(&previous.rectangle));
                    }

                    allocations.push(alloc);
                }
            }
            Evt::Dealloc(idx) => {
                if !allocations.is_empty() {
                    let idx = idx % allocations.len();

                    atlas.deallocate(allocations[idx].id);
                    allocations.swap_remove(idx);
                }
            }
        }
    }

    for alloc in allocations {
        atlas.deallocate(alloc.id);
    }

    assert!(atlas.is_empty());
    assert_eq!(atlas.allocated_space(), 0);
});
