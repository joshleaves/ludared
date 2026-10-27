// Table k advances a byte through k additional zero bytes of the EDC recurrence.
pub(super) const EDC_TABLES: [[u32; 256]; 4] = {
  let mut tables = [[0u32; 256]; 4];
  let mut index = 0;
  while index < 256 {
    let mut crc = index as u32;
    let mut bit = 0;
    while bit < 8 {
      crc = (crc >> 1) ^ if crc & 1 != 0 { 0xD801_8001 } else { 0 };
      bit += 1;
    }
    tables[0][index] = crc;
    index += 1;
  }
  let mut slice = 1;
  while slice < 4 {
    let mut index = 0;
    while index < 256 {
      let crc = tables[slice - 1][index];
      tables[slice][index] = (crc >> 8) ^ tables[0][(crc & 0xFF) as usize];
      index += 1;
    }
    slice += 1;
  }
  tables
};

pub(super) fn edc(data: &[u8]) -> u32 {
  let mut crc = 0u32;
  let (chunks, remainder) = data.as_chunks::<4>();
  for chunk in chunks {
    let low = (crc ^ u32::from_le_bytes(*chunk)).to_le_bytes();
    crc = EDC_TABLES[3][low[0] as usize]
      ^ EDC_TABLES[2][low[1] as usize]
      ^ EDC_TABLES[1][low[2] as usize]
      ^ EDC_TABLES[0][low[3] as usize];
  }
  for &byte in remainder {
    crc = (crc >> 8) ^ EDC_TABLES[0][((crc ^ u32::from(byte)) & 0xFF) as usize];
  }
  crc
}

pub(super) fn ecc_tables() -> ([u8; 256], [u8; 256]) {
  let mut forward = [0u8; 256];
  let mut backward = [0u8; 256];

  for i in 0..256 {
    let j = ((i << 1) ^ if i & 0x80 != 0 { 0x11D } else { 0 }) as u8;

    forward[i] = j;
    backward[(i as u8 ^ j) as usize] = i as u8;
  }

  (forward, backward)
}

#[inline]
pub(super) fn ecc_block(
  source: &[u8],
  major_count: usize,
  minor_count: usize,
  major_mult: usize,
  minor_inc: usize,
  forward: &[u8; 256],
  backward: &[u8; 256],
) -> Vec<u8> {
  let size = major_count * minor_count;
  let mut result = vec![0u8; major_count * 2];

  // P and Q both interleave codewords in adjacent byte pairs. Share their
  // source-index calculation while updating the two scalar checksums in order.
  for major in (0..major_count).step_by(2) {
    let mut index = (major >> 1) * major_mult;
    let mut a = [0u8; 2];
    let mut b = [0u8; 2];

    for _ in 0..minor_count {
      let values = [
        source[index],
        source[index + 1],
      ];
      index += minor_inc;
      if index >= size {
        index -= size;
      }
      for lane in 0..2 {
        a[lane] = forward[(a[lane] ^ values[lane]) as usize];
        b[lane] ^= values[lane];
      }
    }

    for lane in 0..2 {
      a[lane] = backward[(forward[a[lane] as usize] ^ b[lane]) as usize];
      result[major + lane] = a[lane];
      result[major + lane + major_count] = a[lane] ^ b[lane];
    }
  }

  result
}
