//! Shared raw-sector primitives for the CD-ROM sector codecs.

use super::edc_ecc;

pub(super) const CDROM_SECTOR_MAGIC: [u8; 12] = [
  0x00,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0xFF,
  0x00,
];

pub(super) const MODE1: u8 = 0x01;
#[allow(dead_code)]
pub(super) const MODE2: u8 = 0x02;

fn bcd(value: u32) -> u8 {
  ((value / 10) << 4 | (value % 10)) as u8
}

pub(super) fn msf(frame: u32) -> Result<[u8; 3], String> {
  let minutes = frame / (75 * 60);

  if minutes > 99 {
    return Err("MSF address exceeds 99:59:74".into());
  }

  Ok([
    bcd(minutes),
    bcd((frame / 75) % 60),
    bcd(frame % 75),
  ])
}

#[allow(dead_code)]
pub(super) fn looks_like_xa(sector: &[u8]) -> bool {
  // CD-ROM XA sectors repeat their four-byte subheader twice.
  if sector[16..20] != sector[20..24] {
    return false;
  }

  let submode = sector[18];
  let form2 = submode & 0x20 != 0;
  let edc_offset = if form2 { 2348 } else { 2072 };
  let stored_edc = u32::from_le_bytes(sector[edc_offset..edc_offset + 4].try_into().unwrap());

  stored_edc == edc_ecc::edc(&sector[16..edc_offset])
}
