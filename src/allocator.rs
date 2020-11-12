use std::num::Wrapping;
use std::u16;

use crate::{Size, Rectangle, point2, size2};

/// ID referring to an allocated rectangle.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct AllocId(pub(crate) u32);

impl AllocId {
    pub fn serialize(&self) -> u32 {
        self.0
    }

    pub fn deserialize(bytes: u32) -> Self {
        AllocId(bytes)
    }
}

const BIN_BITS: u32 = 12;
const ITEM_BITS: u32 = 12;
const GEN_BITS: u32 = 8;

const BIN_MASK: u32   = (1 << BIN_BITS) - 1;
const ITEM_MASK: u32  = ((1 << ITEM_BITS) - 1) << BIN_BITS;
const GEN_MASK: u32   = ((1 << GEN_BITS) - 1) << (BIN_BITS + ITEM_BITS);

const MAX_ITEMS_PER_BIN: u16 = (ITEM_MASK >> 12) as u16;
const MAX_BIN_COUNT: usize = BIN_MASK as usize;
const MAX_SHELF_COUNT: usize = u16::MAX as usize;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct BinIndex(u16);

impl BinIndex {
    fn to_usize(self) -> usize {
        self.0 as usize
    }

    const INVALID: Self = BinIndex(u16::MAX);
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct Shelf {
    y: u16,
    height: u16,
    bin_width: u16,

    first_bin: BinIndex,
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct Bin {
    x: u16,
    free_space: u16,

    next: BinIndex,

    /// Bins are cleared when their reference count goes back to zero.
    refcount: u16,
    /// Similar to refcount except that the counter is not decremented
    /// when an item is deallocated. We only use this so that allocation
    /// ids are unique within a bin.
    item_count: u16,
    shelf: u16,
    generation: Wrapping<u8>,
}

/// Options to tweak the behavior of the atlas allocator.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct AllocatorOptions {
    /// Align item sizes to a multiple of this alignment.
    ///
    /// Default value: [1, 1] (no alignment).
    pub alignment: Size,
    /// Use vertical instead of horizontal shelves.
    ///
    /// Default value: false.
    pub vertical_shelves: bool,
}

pub const DEFAULT_OPTIONS: AllocatorOptions = AllocatorOptions {
    vertical_shelves: false,
    alignment: size2(1, 1),
};

impl Default for AllocatorOptions {
    fn default() -> Self {
        DEFAULT_OPTIONS
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct Allocation {
    pub id: AllocId,
    pub rectangle: Rectangle,
}

/// A Shelf-packing dynamic texture atlas allocator, inspired by https://github.com/mapbox/shelf-pack/
///
/// Items are accumulated into bins which are laid out in rows (shelves) of variable height.
/// When allocating we first look for a suitable bin. If none is found, a new shelf of the desired height
/// is pushed.
///
/// Lifetime isn't tracked at item granularity. Instead, items are grouped into bins and deallocation happens
/// per bin when all items of the bins are removed.
/// When the top-most shelf is empty, it is removed, potentially cascading into garbage-collecting the next
/// shelf, etc.
///
/// This allocator works well when there are a lot of items with similar sizes (typically, glyph atlases) 
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct AtlasAllocator {
    shelves: Vec<Shelf>,
    bins: Vec<Bin>,
    available_height: u16,
    width: u16,
    height: u16,
    first_unallocated_bin: BinIndex,
    flip_xy: bool,
    alignment: Size,
}

impl AtlasAllocator {
    /// Create an atlas allocator with provided options.
    pub fn with_options(size: Size, options: &AllocatorOptions) -> Self {
        assert!(size.width < u16::MAX as i32);
        assert!(size.height < u16::MAX as i32);

        let (width, height) = if options.vertical_shelves {
            (size.height as u16, size.width as u16)
        } else {
            (size.width as u16, size.height as u16)
        };

        AtlasAllocator {
            shelves: Vec::new(),
            bins: Vec::new(),
            available_height: height,
            width,
            height,
            first_unallocated_bin: BinIndex::INVALID,
            flip_xy: options.vertical_shelves,
            alignment: options.alignment,
        }
    }

    /// Create an atlas allocator with default options.
    pub fn new(size: Size) -> Self {
        Self::with_options(size, &DEFAULT_OPTIONS)
    }

    pub fn clear(&mut self) {
        self.shelves.clear();
        self.bins.clear();
        self.first_unallocated_bin = BinIndex::INVALID;
    }

    pub fn size(&self) -> Size {
        size2(self.width as i32, self.height as i32)
    }

    pub fn is_empty(&self) -> bool {
        self.shelves.is_empty()
    }

    /// Allocate a rectangle in the atlas.
    pub fn allocate(&mut self, mut requested_size: Size) -> Option<Allocation> {
        if requested_size.is_empty() {
            return None;
        }

        adjust_size(self.alignment.width, &mut requested_size.width);
        adjust_size(self.alignment.height, &mut requested_size.height);

        if requested_size.width > self.width as i32 || requested_size.height > self.height as i32 {
            return None;
        }

        let (w, h) = convert_coordinates(self.flip_xy, requested_size.width as u16, requested_size.height as u16);

        let mut selected_shelf = std::usize::MAX;
        let mut selected_bin = BinIndex::INVALID;
        let mut best_waste = u16::MAX;

        let can_add_shelf = self.available_height >= h
            && self.shelves.len() < MAX_SHELF_COUNT
            && self.bins.len() < MAX_BIN_COUNT;

        'shelves: for (shelf_index, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < h || shelf.bin_width < w {
                continue;
            }

            let y_waste = shelf.height - h;
            if y_waste > best_waste || (can_add_shelf && y_waste > h) {
                continue;
            }

            let mut bin_index = shelf.first_bin;
            while bin_index != BinIndex::INVALID {
                let bin = &self.bins[bin_index.to_usize()];

                if bin.free_space >= w && bin.item_count < MAX_ITEMS_PER_BIN {
                    if y_waste == 0 && bin.free_space == w {
                        selected_shelf = shelf_index;
                        selected_bin = bin_index;

                        break 'shelves;
                    }

                    if y_waste < best_waste {
                        best_waste = y_waste;
                        selected_shelf = shelf_index;
                        selected_bin = bin_index;
                        break;
                    }
                }

                bin_index = bin.next;
            }
        }

        if selected_bin == BinIndex::INVALID {
            if can_add_shelf {
                selected_shelf = self.add_shelf(w, h);
                selected_bin = self.shelves[selected_shelf].first_bin;
            } else {
                // Attempt to merge some empty shelves to make a big enough spot.
                let selected = self.coalesce_shelves(w, h);
                selected_shelf = selected.0;
                selected_bin = selected.1;
            }
        }

        if selected_bin != BinIndex::INVALID {
            return self.alloc_from_bin(selected_shelf, selected_bin, w);
        }

        return  None;
    }

    /// Deallocate a rectangle in the atlas.
    ///
    /// Space is only reclaimed when all items of the same bin are deallocated.
    pub fn deallocate(&mut self, id: AllocId) {
        if self.deallocate_from_bin(id) {
            self.cleanup_shelves();
        }
    }

    fn alloc_from_bin(&mut self, shelf_index: usize, bin_index: BinIndex, width: u16) -> Option<Allocation> {
        let shelf = &mut self.shelves[shelf_index];
        let bin = &mut self.bins[bin_index.to_usize()];

        debug_assert!(bin.free_space >= width);

        let min_x = bin.x + shelf.bin_width - bin.free_space;
        let min_y = shelf.y;
        let max_x = min_x + width;
        let max_y = min_y + shelf.height;

        let (min_x, min_y) = convert_coordinates(self.flip_xy, min_x, min_y);
        let (max_x, max_y) = convert_coordinates(self.flip_xy, max_x, max_y);

        bin.free_space -= width;
        bin.refcount += 1;
        bin.item_count += 1;

        let id = AllocId(
            (bin_index.0 as u32) & BIN_MASK
            | ((bin.item_count as u32) << 12) & ITEM_MASK
            | (bin.generation.0 as u32) << 24
        );

        let rectangle = Rectangle {
            min: point2(min_x as i32, min_y as i32),
            max: point2(max_x as i32, max_y as i32),
        };

        Some(Allocation { id, rectangle })
    }

    fn add_shelf(&mut self, width: u16, height: u16) -> usize {
        let height = shelf_height(height).min(self.available_height);
        let num_bins = self.num_bins(width, height);
        let bin_width = self.height / num_bins;
        let y = self.height - self.available_height;
        self.available_height -= height;

        let shelf_index = self.shelves.len();

        // Initialize the bins for our new shelf.
        let mut x = 0;
        let mut bin_next = BinIndex::INVALID;
        for _ in 0..num_bins {
            let mut bin = Bin {
                next: bin_next,
                x,
                free_space: bin_width,
                refcount: 0,
                shelf: shelf_index as u16,
                generation: Wrapping(0),
                item_count: 0,
            };

            let mut bin_index = self.first_unallocated_bin;
            x += bin_width;

            if bin_index == BinIndex::INVALID {
                bin_index = BinIndex(self.bins.len() as u16);
                self.bins.push(bin);
            } else {
                let idx = bin_index.to_usize();
                bin.generation = self.bins[idx].generation + Wrapping(1);
                self.first_unallocated_bin = self.bins[idx].next;

                self.bins[idx] = bin;
            }

            bin_next = bin_index;
        }

        self.shelves.push(Shelf {
            y,
            height,
            bin_width,
            first_bin: bin_next,
        });

        shelf_index
    }

    /// Find a sequence of consecutive shelves that can be coalesced into a single one
    /// tall enough to fit the provided size.
    ///
    /// If such a sequence is found, grow the height of first shelf and squash the other
    /// ones to zero.
    /// The squashed shelves are not removed, their height is just set to zero so no item
    /// can go in, and they will be garbage-collected whenever there's no shelf above them.
    /// For simplicity, the bin width is not modified.
    fn coalesce_shelves(&mut self, w: u16, h: u16) -> (usize, BinIndex) {
        let len = self.shelves.len();
        let mut coalesce_range = None;
        let mut coalesced_height = 0;

        'outer: for shelf_index in 0..len {
            if self.shelves[shelf_index].bin_width < w {
                continue;
            }
            if !self.shelf_is_empty(shelf_index) {
                continue;
            }

            coalesced_height = self.shelves[shelf_index].height;
            for i in 1..3 {
                if shelf_index + i >= len {
                    break 'outer;
                }

                if !self.shelf_is_empty(shelf_index + i) {
                    continue 'outer;
                }

                coalesced_height += self.shelves[shelf_index + i].height;

                if coalesced_height >= h {
                    coalesce_range = Some(shelf_index .. (shelf_index + i + 1));
                    break 'outer;
                }
            }
        }

        if let Some(range) = coalesce_range {
            for i in range.start + 1 .. range.end {
                self.shelves[i].height = 0;
            }

            let shelf_index = range.start;
            let shelf = &mut self.shelves[shelf_index];
            shelf.height = coalesced_height;

            return (shelf_index, shelf.first_bin);
        }

        (0, BinIndex::INVALID)
    }

    fn num_bins(&self, width: u16, height: u16) -> u16 {
        match self.width / u16::max(width, height) {
            0 ..= 4 => 1,
            5 ..= 15 => 2,
            16 ..= 64 => 4,
            65 ..= 256 => 8,
            _ => 16,
        }.min((MAX_BIN_COUNT - self.bins.len()) as u16)
    }

    /// Returns true if we should garbage-collect the shelves as a result of
    /// removing this element (we deallocated the last item from the bin on
    /// the top-most shelf).
    fn deallocate_from_bin(&mut self, id: AllocId) -> bool {
        let bin_index = (id.0 & BIN_MASK) as usize;
        let generation = ((id.0 & GEN_MASK) >> 24 ) as u8;

        let bin = &mut self.bins[bin_index];

        let expected_generation = bin.generation.0;
        assert_eq!(generation, expected_generation);

        assert!(bin.refcount > 0);
        bin.refcount -= 1;

        let shelf = &self.shelves[bin.shelf as usize];

        let bin_is_empty = bin.refcount == 0;
        if bin_is_empty {
            bin.free_space = shelf.bin_width;
        }

        bin_is_empty && bin.shelf as usize == self.shelves.len() - 1
    }

    fn cleanup_shelves(&mut self) {
        while self.shelves.len() > 0 {
            {
                let shelf = self.shelves.last().unwrap();
                let mut bin_index = shelf.first_bin;
                let mut last_bin = shelf.first_bin;

                while bin_index != BinIndex::INVALID {
                    let bin = &self.bins[bin_index.to_usize()];

                    if bin.refcount != 0 {
                        return;
                    }

                    last_bin = bin_index;
                    bin_index = bin.next;
                }

                // We didn't run into any bin on this shelf with live elements,
                // this means we can remove it.

                // Can't have a shelf with no bins.
                debug_assert!(last_bin != BinIndex::INVALID);
                // Add the bins to the free list.
                self.bins[last_bin.to_usize()].next = self.first_unallocated_bin;
                self.first_unallocated_bin = shelf.first_bin;

                // Reclaim the height of the bin. 
                self.available_height += shelf.height;
            }

            self.shelves.pop();
        }
    }

    fn shelf_is_empty(&self, idx: usize) -> bool {
        let shelf = &self.shelves[idx];
        let mut bin_index = shelf.first_bin;

        while bin_index != BinIndex::INVALID {
            let bin = &self.bins[bin_index.to_usize()];

            if bin.refcount != 0 {
                return false;
            }

            bin_index = bin.next;
        }

        true
    }
}

fn convert_coordinates(flip_xy: bool, x: u16, y: u16) -> (u16, u16) {
    if flip_xy {
        (y, x)
    } else {
        (x, y)
    }
}


fn shelf_height(mut size: u16) -> u16 {
    let alignment = match size {
        0 ..= 31 => 8,
        32 ..= 127 => 16,
        128 ..= 511 => 32,
        _ => 64,
    };

    let rem = size % alignment;
    if rem > 0 {
        size += alignment - rem;
    }

    size
}


/// Dump a visual representation of the atlas in SVG format.
pub fn dump_svg(atlas: &AtlasAllocator, output: &mut dyn std::io::Write) -> std::io::Result<()> {
    use svg_fmt::*;

    writeln!(
        output,
        "{}",
        BeginSvg {
            w: atlas.width as f32,
            h: atlas.height as f32
        }
    )?;

    dump_into_svg(atlas, None, output)?;

    writeln!(output, "{}", EndSvg)
}

/// Dump a visual representation of the atlas in SVG, omitting the beginning and end of the
/// SVG document, so that it can be included in a larger document.
///
/// If a rectangle is provided, translate and scale the output to fit it.
pub fn dump_into_svg(atlas: &AtlasAllocator, rect: Option<&Rectangle>, output: &mut dyn std::io::Write) -> std::io::Result<()> {
    use svg_fmt::*;

    let (sx, sy, tx, ty) = if let Some(rect) = rect {
        (
            rect.size().width as f32 / atlas.width as f32,
            rect.size().height as f32 / atlas.height as f32,
            rect.min.x as f32,
            rect.min.y as f32,
        )
    } else {
        (1.0, 1.0, 0.0, 0.0)        
    };

    writeln!(
        output,
        r#"    {}"#,
        rectangle(tx, ty, atlas.width as f32 * sx, atlas.height as f32 * sy)
            .fill(rgb(40, 40, 40))
            .stroke(Stroke::Color(black(), 1.0))
    )?;


    for shelf in &atlas.shelves {
        let mut bin_index = shelf.first_bin;

        let y = shelf.y as f32 * sy + ty;
        let h = shelf.height as f32 * sy;
        while bin_index != BinIndex::INVALID {
            let bin = &atlas.bins[bin_index.to_usize()];

            let x = bin.x as f32 * sx + tx;
            let w = (shelf.bin_width - bin.free_space) as f32 * sx;

            writeln!(
                output,
                r#"    {}"#,
                rectangle(x, y, w, h)
                    .fill(rgb(70, 70, 180))
                    .stroke(Stroke::Color(black(), 1.0))
            )?;

            if bin.free_space > 0 {
                let x_free = x + w;
                let w_free = bin.free_space as f32 * sx;

                writeln!(
                    output,
                    r#"    {}"#,
                    rectangle(x_free, y, w_free, h)
                        .fill(rgb(50, 50, 50))
                        .stroke(Stroke::Color(black(), 1.0))
                )?;
            }

            bin_index = bin.next;
        }
    }

    Ok(())
}

fn adjust_size(alignment: i32, size: &mut i32) {
    let rem = *size % alignment;
    if rem > 0 {
        *size += alignment - rem;
    }
}

#[test]
fn atlas_basic() {
    let mut atlas = AtlasAllocator::new(size2(1000, 1000));

    let full = atlas.allocate(size2(1000, 1000)).unwrap().id;
    assert!(atlas.allocate(size2(1, 1)).is_none());

    atlas.deallocate(full);

    let a = atlas.allocate(size2(10, 10)).unwrap().id;
    let b = atlas.allocate(size2(50, 30)).unwrap().id;
    let c = atlas.allocate(size2(12, 45)).unwrap().id;
    let d = atlas.allocate(size2(60, 45)).unwrap().id;
    let e = atlas.allocate(size2(1, 1)).unwrap().id;
    let f = atlas.allocate(size2(128, 128)).unwrap().id;
    let g = atlas.allocate(size2(256, 256)).unwrap().id;

    atlas.deallocate(b);
    atlas.deallocate(f);
    atlas.deallocate(c);
    atlas.deallocate(e);
    let h = atlas.allocate(size2(500, 200)).unwrap().id;
    atlas.deallocate(a);
    let i = atlas.allocate(size2(500, 200)).unwrap().id;
    atlas.deallocate(g);
    atlas.deallocate(h);
    atlas.deallocate(d);
    atlas.deallocate(i);

    let full = atlas.allocate(size2(1000, 1000)).unwrap().id;
    assert!(atlas.allocate(size2(1, 1)).is_none());
    atlas.deallocate(full);
}

#[test]
fn fuzz_01() {
    let mut atlas = AtlasAllocator::new(size2(1000, 1000));

    assert!(atlas.allocate(size2(65280, 1)).is_none());
    assert!(atlas.allocate(size2(1, 65280)).is_none());
}

#[test]
fn test_coalesce_shelves() {
    let mut atlas = AtlasAllocator::new(size2(256, 256));

    // Allocate 7 shelves (leaving 32px of remaining space on top).
    let mut ids = Vec::new();
    for _ in 0..7 {
        for _ in 0..8 {
            ids.push(atlas.allocate(size2(32, 32)).unwrap().id)
        }
    }

    // Free the first shelf.
    for i in 0..8 {
        atlas.deallocate(ids[i]);
    }

    // Free the 3rd and 4th shelf.
    for i in 16..32 {
        atlas.deallocate(ids[i]);
    }

    // Not enough space left in existing shelves and above.
    // even coalescing is not sufficient.
    assert!(atlas.allocate(size2(70, 70)).is_none());

    // Not enough space left in existing shelves and above.
    // The 3rd and 4th row can be coalesced to fit this allocation, though.
    let id = atlas.allocate(size2(64, 64)).unwrap().id;

    // Deallocate everything
    for i in 8..16 {
        atlas.deallocate(ids[i]);
    }

    atlas.deallocate(id);

    for i in 32..56 {
        atlas.deallocate(ids[i]);
    }

    //dump_svg(&atlas, &mut std::fs::File::create("tmp.svg").expect("!!"));

    assert!(atlas.is_empty());
}
