#[cfg(feature = "serde")]
#[macro_use]
pub extern crate serde;
pub extern crate euclid;

mod bucketed;
pub mod allocator2;

pub use bucketed::*;
pub use euclid::{point2, size2};

pub type Point = euclid::default::Point2D<i32>;
pub type Size = euclid::default::Size2D<i32>;
pub type Rectangle = euclid::default::Box2D<i32>;

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
    /// If possible split the allocator's surface into multiple columns.
    ///
    /// Having multiple columns allows having more (smaller shelves).
    ///
    /// Default value: 1.
    pub num_columns: i32,
}

pub const DEFAULT_OPTIONS: AllocatorOptions = AllocatorOptions {
    vertical_shelves: false,
    alignment: size2(1, 1),
    num_columns: 1,
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

