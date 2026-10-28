//! Media-unit sizing shared by Nintendo 3DS containers and NCCH regions.
//!
//! The 3DS measures container sizes in *media units*. One media unit is 0x200
//! bytes, or a power-of-two multiple of that where a container's flags say so.

/// The size of one media unit before a container scales it.
pub(crate) const MEDIA_UNIT: u64 = 0x200;

/// The size of one media unit for a media-unit-size flag byte.
///
/// A container records `MediaUnitSize = 0x200 * 2^flag`, so the flag is the power
/// of two the unit is scaled by. `None` when the scaled unit overflows.
pub(crate) fn media_unit_size(flag: u8) -> Option<u64> {
  MEDIA_UNIT.checked_mul(1u64.checked_shl(u32::from(flag))?)
}
