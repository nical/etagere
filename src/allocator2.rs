use crate::{AllocId, Allocation, AllocatorOptions, DEFAULT_OPTIONS, Size, Rectangle, point2};

const SHELF_SPLIT_THRESHOLD: u16 = 8;
const ITEM_SPLIT_THRESHOLD: u16 = 8;

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct ShelfIndex(u16);

impl ShelfIndex {
    const NONE: Self = ShelfIndex(std::u16::MAX);

    fn index(self) -> usize { self.0 as usize }

    fn is_some(self) -> bool { self.0 != std::u16::MAX }

    fn is_none(self) -> bool { self.0 == std::u16::MAX }
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct ItemIndex(u16);

impl ItemIndex {
    const NONE: Self = ItemIndex(std::u16::MAX);

    fn index(self) -> usize { self.0 as usize }

    fn is_some(self) -> bool { self.0 != std::u16::MAX }

    fn is_none(self) -> bool { self.0 == std::u16::MAX }
}

#[derive(Clone)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct Shelf {
    y: u16,
    height: u16,
    prev: ShelfIndex,
    next: ShelfIndex,
    first_item: ItemIndex,
    is_empty: bool,
}

#[derive(Clone)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
struct Item {
    x: u16,
    width: u16,
    prev: ItemIndex,
    next: ItemIndex,
    shelf: ShelfIndex,
    allocated: bool,
}

// Note: if allocating is slow we can use the guillotiere trick of storing multiple lists of free
// rects (per shelf height) instead of iterating the shelves and items.

/// A shelf-packing dynamic atlas allocator tracking each allocation individually and with support
/// for coalescing empty shelves.
#[derive(Clone)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct AtlasAllocator {
    shelves: Vec<Shelf>,
    items: Vec<Item>,
    alignment: Size,
    flip_xy: bool,
    size: Size,
    first_shelf: ShelfIndex,
    free_items: ItemIndex,
    free_shelves: ShelfIndex,
}

impl AtlasAllocator {
    /// Create an atlas allocator with provided options.
    pub fn with_options(size: Size, options: &AllocatorOptions) -> Self {
        assert!(size.width > 0);
        assert!(size.height > 0);
        assert!(size.width <= std::u16::MAX as i32);
        assert!(size.height <= std::u16::MAX as i32);
        assert!(options.alignment.width > 0);
        assert!(options.alignment.height > 0);

        let first_shelf = ShelfIndex(0);
        let first_item = ItemIndex(0);

        AtlasAllocator {
            shelves: vec![Shelf {
                y: 0,
                height: size.height as u16,
                prev: ShelfIndex::NONE,
                next: ShelfIndex::NONE,
                is_empty: true,
                first_item,
            }],
            items: vec![Item {
                x: 0,
                width: size.width as u16,
                prev: ItemIndex::NONE,
                next: ItemIndex::NONE,
                shelf: first_shelf,
                allocated: false,
            }],
            size,
            alignment: options.alignment,
            flip_xy: options.vertical_shelves,
            first_shelf,
            free_items: ItemIndex::NONE,
            free_shelves: ShelfIndex::NONE,
        }
    }

    /// Create an atlas allocator with default options.
    pub fn new(size: Size) -> Self {
        Self::with_options(size, &DEFAULT_OPTIONS)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.shelves.clear();

        let first_shelf = ShelfIndex(0);
        let first_item = ItemIndex(0);

        self.shelves.push(Shelf {
            y: 0,
            height: self.size.height as u16,
            prev: ShelfIndex::NONE,
            next: ShelfIndex::NONE,
            is_empty: true,
            first_item,
        });

        self.items.push(Item {
            x: 0,
            width: self.size.width as u16,
            prev: ItemIndex::NONE,
            next: ItemIndex::NONE,
            shelf: first_shelf,
            allocated: false,
        });

        self.first_shelf = first_shelf;

        self.free_shelves = ShelfIndex::NONE;
        self.free_items = ItemIndex::NONE;
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Allocate a rectangle in the atlas.
    pub fn allocate(&mut self, mut size: Size) -> Option<Allocation> {
        if size.is_empty() {
            return None;
        }

        adjust_size(self.alignment.width, &mut size.width);
        adjust_size(self.alignment.height, &mut size.height);

        if size.width > self.size.width || size.height > self.size.height {
            return None;
        }

        let (width, height) = convert_coordinates(self.flip_xy, size.width as u16, size.height as u16);
        let height = shelf_height(height);

        let mut selected_shelf_height = std::u16::MAX;
        let mut selected_shelf = ShelfIndex::NONE;
        let mut selected_item = ItemIndex::NONE;
        let mut shelf_idx = self.first_shelf;

        while shelf_idx.is_some() {
            let shelf = &self.shelves[shelf_idx.index()];

            if shelf.height < height
                || shelf.height >= selected_shelf_height
                || (!shelf.is_empty && shelf.height > height * 2) {
                shelf_idx = shelf.next;
                continue;
            }

            let mut item_idx = shelf.first_item;
            while item_idx.is_some() {
                let item = &self.items[item_idx.index()];
                if !item.allocated && item.width > width {
                    break;
                }

                item_idx = item.next;
            }

            if item_idx.is_some() {
                selected_shelf = shelf_idx;
                selected_shelf_height = shelf.height;
                selected_item = item_idx;
    
                if shelf.height == height {
                    // Perfect fit, stop searching.
                    break;
                }
            }

            shelf_idx = shelf.next;
        }


        if selected_shelf.is_none() {
            return None;
        }

        let shelf = self.shelves[selected_shelf.index()].clone();
        if shelf.is_empty {
            self.shelves[selected_shelf.index()].is_empty = false;
        }

        if shelf.is_empty && shelf.height > height + SHELF_SPLIT_THRESHOLD {
            // Split the empty shelf into one of the desired size and a new
            // empty one with a single empty item.

            let new_shelf_idx =  self.add_shelf(Shelf {
                y: shelf.y + height,
                height: shelf.height - height,
                prev: selected_shelf,
                next: shelf.next,
                first_item: ItemIndex::NONE,
                is_empty: true,
            });

            let new_item_idx = self.add_item(Item {
                x: 0,
                width: self.size.width as u16,
                prev: ItemIndex::NONE,
                next: ItemIndex::NONE,
                shelf: new_shelf_idx,
                allocated: false,
            });

            self.shelves[new_shelf_idx.index()].first_item = new_item_idx;

            let next = self.shelves[selected_shelf.index()].next;
            self.shelves[selected_shelf.index()].height = height;
            self.shelves[selected_shelf.index()].next = new_shelf_idx;

            if next.is_some() {
                self.shelves[next.index()].prev = new_shelf_idx;
            }
        }

        let item = self.items[selected_item.index()].clone();

        if item.width - width > ITEM_SPLIT_THRESHOLD {

            let new_item_idx = self.add_item(Item {
                x: item.x + width,
                width: item.width - width,
                prev: selected_item,
                next: item.next,
                shelf: item.shelf,
                allocated: false,
            });

            self.items[selected_item.index()].width = width;
            self.items[selected_item.index()].next = new_item_idx;

            if item.next.is_some() {
                self.items[item.next.index()].prev = new_item_idx;
            }
        }

        self.items[selected_item.index()].allocated = true;

        let x0 = item.x;
        let y0 = shelf.y;
        let x1 = x0 + width;
        let y1 = y0 + height;

        let (x0, y0) = convert_coordinates(self.flip_xy, x0, y0);
        let (x1, y1) = convert_coordinates(self.flip_xy, x1, y1);

        self.check();

        Some(Allocation {
            id: AllocId(selected_item.0 as u32),
            rectangle: Rectangle {
                min: point2(x0 as i32, y0 as i32),
                max: point2(x1 as i32, y1 as i32),
            },
        })
    }

    /// Deallocate a rectangle in the atlas.
    pub fn deallocate(&mut self, id: AllocId) {
        let item_idx = ItemIndex(id.0 as u16);

        let item = self.items[item_idx.index()].clone();
        let Item { mut prev, mut next, mut width, allocated, .. } = self.items[item_idx.index()];
        assert!(allocated);

        self.items[item_idx.index()].allocated = false;

        if next.is_some() && !self.items[next.index()].allocated {
            // Merge the next item into this one.

            let next_next = self.items[next.index()].next;
            let next_width = self.items[next.index()].width;

            self.items[item_idx.index()].next = next_next;
            self.items[item_idx.index()].width += next_width;
            width = self.items[item_idx.index()].width;

            if next_next.is_some() {
                self.items[next_next.index()].prev = item_idx;
            }

            // Add next to the free list.
            self.remove_item(next);

            next = next_next
        }

        if prev.is_some() && !self.items[prev.index()].allocated {
            // Merge the item into the previous one.

            self.items[prev.index()].next = next;
            self.items[prev.index()].width += width;

            if next.is_some() {
                self.items[next.index()].prev = prev;
            }

            // Add item_idx to the free list.
            self.remove_item(item_idx);

            prev = self.items[prev.index()].prev;
        }

        if prev.is_none() && next.is_none() {
            let shelf_idx = item.shelf;
            // The shelf is now empty. 
            self.shelves[shelf_idx.index()].is_empty = true;

            let next_shelf = self.shelves[shelf_idx.index()].next;
            if next_shelf.is_some() && self.shelves[next_shelf.index()].is_empty {
                // Merge the next shelf into this one.

                let next_next = self.shelves[next_shelf.index()].next;
                let next_height = self.shelves[next_shelf.index()].height;

                self.shelves[shelf_idx.index()].next = next_next;
                self.shelves[shelf_idx.index()].height += next_height;

                if next_next.is_some() {
                    self.shelves[next_next.index()].prev = shelf_idx;
                }

                // Add next to the free list.
                self.remove_shelf(next_shelf);
            }

            let prev_shelf = self.shelves[shelf_idx.index()].prev;
            if prev_shelf.is_some() && self.shelves[prev_shelf.index()].is_empty {
                // Merge the shelf into the previous one.

                let next_shelf = self.shelves[shelf_idx.index()].next;
                self.shelves[prev_shelf.index()].next = next_shelf;
                self.shelves[prev_shelf.index()].height += self.shelves[shelf_idx.index()].height;

                self.shelves[prev_shelf.index()].next = self.shelves[shelf_idx.index()].next;
                if next_shelf.is_some() {
                    self.shelves[next_shelf.index()].prev = prev_shelf;
                }

                // Add the shelf to the free list.
                self.remove_shelf(shelf_idx);
            }
        }

        self.check();
    }

    pub fn is_empty(&self) -> bool {
        let shelf = &self.shelves[self.first_shelf.index()];
        let item = &self.items[shelf.first_item.index()];

        shelf.next.is_none() && item.next.is_none() && !item.allocated
    }

    fn remove_item(&mut self, idx: ItemIndex) {
        self.items[idx.index()].next = self.free_items;
        self.free_items = idx;
    }

    fn remove_shelf(&mut self, idx: ShelfIndex) {
        // Remove the shelf's item.
        self.remove_item(self.shelves[idx.index()].first_item);

        self.shelves[idx.index()].next = self.free_shelves;
        self.free_shelves = idx;
    }

    fn add_item(&mut self, item: Item) -> ItemIndex {
        if self.free_items.is_some() {
            let idx = self.free_items;
            self.free_items = self.items[idx.index()].next;
            self.items[idx.index()] = item;

            return idx;
        }

        let idx = ItemIndex(self.items.len() as u16);
        self.items.push(item);

        idx
    }

    fn add_shelf(&mut self, shelf: Shelf) -> ShelfIndex {
        if self.free_shelves.is_some() {
            let idx = self.free_shelves;
            self.free_shelves = self.shelves[idx.index()].next;
            self.shelves[idx.index()] = shelf;

            return idx;
        }

        let idx = ShelfIndex(self.shelves.len() as u16);
        self.shelves.push(shelf);

        idx
    }

    fn check(&self) {
        let (target_w, target_h) = if self.flip_xy {
            (self.size.height, self.size.width)
        } else {
            (self.size.width, self.size.height)
        };

        let mut prev_empty = false;
        let mut accum_h = 0;
        let mut shelf_idx = self.first_shelf;
        while shelf_idx.is_some() {
            let shelf = &self.shelves[shelf_idx.index()];
            accum_h += shelf.height;
            if prev_empty {
                assert!(!shelf.is_empty);
            }
            if shelf.is_empty {
                assert!(!self.items[shelf.first_item.index()].allocated);
                assert!(self.items[shelf.first_item.index()].next.is_none());
            }
            prev_empty = shelf.is_empty;

            let mut accum_w = 0;
            let mut prev_allocated = true;
            let mut item_idx = shelf.first_item;
            let mut prev_item_idx = ItemIndex::NONE;
            while item_idx.is_some() {
                let item = &self.items[item_idx.index()];
                accum_w += item.width;

                assert_eq!(item.prev, prev_item_idx);

                if !prev_allocated {
                    assert!(item.allocated, "item {:?} should be allocated", item_idx.0);
                }
                prev_allocated = item.allocated;

                prev_item_idx = item_idx;
                item_idx = item.next;
            }

            assert_eq!(accum_w as i32, target_w);

            shelf_idx = shelf.next;
        }

        assert_eq!(accum_h as i32, target_h);
    }
}


/// Dump a visual representation of the atlas in SVG format.
pub fn dump_svg(atlas: &AtlasAllocator, output: &mut dyn std::io::Write) -> std::io::Result<()> {
    use svg_fmt::*;

    writeln!(
        output,
        "{}",
        BeginSvg {
            w: atlas.size.width as f32,
            h: atlas.size.height as f32
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
            rect.size().width as f32 / atlas.size.width as f32,
            rect.size().height as f32 / atlas.size.height as f32,
            rect.min.x as f32,
            rect.min.y as f32,
        )
    } else {
        (1.0, 1.0, 0.0, 0.0)        
    };

    writeln!(
        output,
        r#"    {}"#,
        rectangle(tx, ty, atlas.size.width as f32 * sx, atlas.size.height as f32 * sy)
            .fill(rgb(40, 40, 40))
            .stroke(Stroke::Color(black(), 1.0))
    )?;

    let mut shelf_idx = atlas.first_shelf;
    while shelf_idx.is_some() {
        let shelf = &atlas.shelves[shelf_idx.index()];

        let y = shelf.y as f32 * sy + ty;
        let h = shelf.height as f32 * sy;

        let mut item_idx = shelf.first_item;
        while item_idx.is_some() {
            let item = &atlas.items[item_idx.index()];

            let x = item.x as f32 * sx + tx;
            let w = item.width as f32 * sx;

            let color = if item.allocated {
                rgb(70, 70, 180)
            } else {
                rgb(50, 50, 50)
            };

            let (x, y) = if atlas.flip_xy { (y, x) } else { (x, y) };
            let (w, h) = if atlas.flip_xy { (h, w) } else { (w, h) };

            writeln!(
                output,
                r#"    {}"#,
                rectangle(x, y, w, h).fill(color).stroke(Stroke::Color(black(), 1.0))
            )?;

            item_idx = item.next;
        }

        shelf_idx = shelf.next;
    }

    Ok(())
}

fn adjust_size(alignment: i32, size: &mut i32) {
    let rem = *size % alignment;
    if rem > 0 {
        *size += alignment - rem;
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

#[test]
fn test_simple() {
    use crate::size2;

    let mut atlas = AtlasAllocator::new(size2(1000, 1000));
    assert!(atlas.is_empty());

    let a1 = atlas.allocate(size2(20, 30)).unwrap();
    let a2 = atlas.allocate(size2(30, 40)).unwrap();
    let a3 = atlas.allocate(size2(20, 30)).unwrap();

    assert!(a1.id != a2.id);
    assert!(a1.id != a3.id);
    assert!(!atlas.is_empty());

    //dump_svg(&atlas, &mut std::fs::File::create("tmp.svg").expect("!!")).unwrap();

    atlas.deallocate(a1.id);
    atlas.deallocate(a2.id);
    atlas.deallocate(a3.id);

    assert!(atlas.is_empty());
}

#[test]
fn test_options() {
    use crate::size2;

    let alignment = size2(8, 16);

    let mut atlas = AtlasAllocator::with_options(
        size2(1000, 1000),
        &AllocatorOptions {
            alignment,
            vertical_shelves: true,
        },
    );
    assert!(atlas.is_empty());

    let a1 = atlas.allocate(size2(20, 30)).unwrap();
    let a2 = atlas.allocate(size2(30, 40)).unwrap();
    let a3 = atlas.allocate(size2(20, 30)).unwrap();

    assert!(a1.id != a2.id);
    assert!(a1.id != a3.id);
    assert!(!atlas.is_empty());

    assert_eq!(a1.rectangle.min.x % alignment.width, 0);
    assert_eq!(a1.rectangle.min.y % alignment.height, 0);
    assert_eq!(a2.rectangle.min.x % alignment.width, 0);
    assert_eq!(a2.rectangle.min.y % alignment.height, 0);
    assert_eq!(a3.rectangle.min.x % alignment.width, 0);
    assert_eq!(a3.rectangle.min.y % alignment.height, 0);

    assert!(a1.rectangle.size().width >= 20);
    assert!(a1.rectangle.size().height >= 30);
    assert!(a2.rectangle.size().width >= 30);
    assert!(a2.rectangle.size().height >= 40);
    assert!(a3.rectangle.size().width >= 20);
    assert!(a3.rectangle.size().height >= 30);


    dump_svg(&atlas, &mut std::fs::File::create("tmp.svg").expect("!!")).unwrap();

    atlas.deallocate(a1.id);
    atlas.deallocate(a2.id);
    atlas.deallocate(a3.id);

    assert!(atlas.is_empty());
}
