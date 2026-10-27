//! PRS compression with two-byte candidate indexing and dynamic programming.
//!
//! Contiguous candidate storage was inspired by Sewer56's `prs-rs` dictionary:
//! <https://github.com/Sewer56/prs-rs>. This is an independent safe-Rust
//! implementation; the candidate order and chain budget are unchanged.

const MAX_DISTANCE: usize = 8191;
const MAX_LENGTH: usize = 256;

// Control bits + whole payload bytes. Only the final control byte is padded.
const LITERAL_BITS: usize = 1 + 8;
const SHORT_BITS: usize = 4 + 8;
const LONG_BITS: usize = 2 + 16;
const EXTENDED_LONG_BITS: usize = 2 + 24;
const END_BITS: usize = 2 + 16;

#[derive(Clone, Copy)]
enum MatchKind {
  Literal,
  Short,
  Long,
}

#[derive(Clone, Copy)]
struct Match {
  kind: MatchKind,
  length: usize,
  distance: usize,
}

impl Default for Match {
  fn default() -> Self {
    Self {
      kind: MatchKind::Literal,
      length: 1,
      distance: 0,
    }
  }
}

pub fn prs_compress(data: &[u8], chain: usize) -> Vec<u8> {
  let n = data.len();

  if n == 0 {
    return vec![0x02, 0x00, 0x00];
  }

  // Pass 1: Find the longest matches at each position.
  //
  // Group positions by their two-byte key, in ascending input order.
  // Searching a contiguous slice avoids chasing a linked chain while keeping
  // exactly the same candidates and newest-first traversal.
  let key_at = |p: usize| ((data[p] as usize) << 8) | data[p + 1] as usize;
  let mut starts = vec![0usize; 65536];
  for p in 0..n - 1 {
    starts[key_at(p)] += 1;
  }
  let mut total = 0;
  for count in &mut starts {
    let next = total + *count;
    *count = total;
    total = next;
  }
  let mut head = starts.clone();
  let mut positions = vec![0usize; n - 1];
  for p in 0..n - 1 {
    let key = key_at(p);
    positions[head[key]] = p;
    head[key] += 1;
  }
  head.copy_from_slice(&starts);

  // Only the first MAX_LENGTH bytes of a run can contribute to a PRS match.
  let mut runs = vec![1u16; n];
  for p in (0..n - 1).rev() {
    if data[p] == data[p + 1] {
      runs[p] = (runs[p + 1] + 1).min(MAX_LENGTH as u16);
    }
  }

  // A candidate can match beyond a repeated-byte prefix only if its run ends
  // at the same relative position. Index by both byte and remaining run length.
  let run_key = |p: usize| (data[p] as usize) * 256 + runs[p] as usize - 1;
  let mut run_starts = vec![0usize; 65536];
  for p in 0..n {
    run_starts[run_key(p)] += 1;
  }
  let mut total = 0;
  for count in &mut run_starts {
    let next = total + *count;
    *count = total;
    total = next;
  }
  let mut run_head = run_starts.clone();
  let mut run_positions = vec![0usize; n];
  for p in 0..n {
    let key = run_key(p);
    run_positions[run_head[key]] = p;
    run_head[key] += 1;
  }
  run_head.copy_from_slice(&run_starts);
  let mut match_end = vec![0usize; MAX_DISTANCE + 1];

  let mut max_long = vec![0usize; n];
  let mut dist_long = vec![0usize; n];
  let mut max_short = vec![0usize; n];
  let mut dist_short = vec![0usize; n];

  for p in 0..n {
    let mut best_len = 0;
    let mut best_dist = 0;
    let mut short_len = 0;
    let mut short_dist = 0;

    if p + 1 < n {
      let limit = MAX_LENGTH.min(n - p);
      let key = key_at(p);
      let end = head[key];
      let start = starts[key].max(end.saturating_sub(chain));

      // Filtering must not expand the original two-byte search budget.
      let minimum = if start < end {
        positions[start].max(p.saturating_sub(MAX_DISTANCE))
      } else {
        p
      };
      let in_run = chain > 0 && p > 0 && data[p - 1] == data[p] && runs[p] >= 2;
      let candidates = if in_run {
        // Distance 1 matches the whole remaining run and is the nearest
        // possible short match too. Unequal-length runs cannot improve either
        // result; equal-length runs may extend arbitrarily far, up to limit.
        best_len = runs[p] as usize;
        best_dist = 1;
        short_len = best_len;
        short_dist = 1;
        let key = run_key(p);
        if best_len == limit {
          &[][..]
        } else {
          &run_positions[run_starts[key]..run_head[key]]
        }
      } else {
        &positions[start..end]
      };

      for &candidate in candidates.iter().rev() {
        if candidate < minimum {
          break;
        }

        let distance = p - candidate;

        // A candidate must improve either the long or short match.
        let threshold = if distance <= 256 {
          best_len.min(short_len)
        } else {
          best_len
        };

        if threshold < limit && data[candidate + threshold] == data[p + threshold] {
          // Advancing both positions preserves the already compared prefix at
          // this distance. Only compare bytes beyond the last verified end.
          // The run index additionally proves the repeated-byte prefix matches.
          let prefix = if in_run { runs[p] as usize } else { 2 };
          let mut length = match_end[distance].saturating_sub(p).max(prefix).min(limit);

          // Compare 8 bytes at a time.
          while length + 8 <= limit {
            let a = &data[candidate + length..candidate + length + 8];
            let b = &data[p + length..p + length + 8];

            if a != b {
              break;
            }

            length += 8;
          }

          // Finish byte by byte.
          while length < limit && data[candidate + length] == data[p + length] {
            length += 1;
          }

          match_end[distance] = p + length;

          if length > best_len {
            best_len = length;
            best_dist = distance;
          }

          if length > short_len && distance <= 256 {
            short_len = length;
            short_dist = distance;
          }
        }

        if best_len == limit && short_len >= 5.min(limit) {
          break;
        }
      }

      head[key] += 1;
    }

    run_head[run_key(p)] += 1;
    max_long[p] = best_len;
    dist_long[p] = best_dist;

    if short_len >= 2 {
      max_short[p] = short_len.min(5);
      dist_short[p] = short_dist;
    }
  }

  // Pass 2: Dynamic programming.
  //
  // Find the cheapest encoding for each suffix among the retained match lengths.
  //
  // Costs are measured in bits:
  //   Literal: 9
  //   Short:   12
  //   Long:    18 (length 3..9)
  //            26 (length 10..256)
  //   End:     18 (two control bits and a zero word)
  //
  // For P payload bytes and C control bits, serialized size is
  // P + ceil(C / 8) = ceil((8*P + C) / 8). Control bytes are shared across
  // instructions, even when an instruction straddles a control-byte boundary.
  // Thus minimizing these additive bit costs also minimizes serialized bytes;
  // tracking the eight control-bit phases would not improve the minimum size.
  // The search is still limited by the chain budget and the lengths considered
  // below: maximal short, min(long, 9), and maximal long, plus a literal.

  let mut cost = vec![0usize; n + 1];
  cost[n] = END_BITS;
  let mut matches = vec![Match::default(); n];

  for p in (0..n).rev() {
    let mut best_bits = LITERAL_BITS + cost[p + 1];
    let mut best = Match::default();

    let short = max_short[p];

    if short > 0 {
      let bits = SHORT_BITS + cost[p + short];

      if bits < best_bits {
        best_bits = bits;
        best = Match {
          kind: MatchKind::Short,
          length: short,
          distance: dist_short[p],
        };
      }
    }

    let long = max_long[p];

    if long >= 3 {
      let mid = long.min(9);
      let bits = LONG_BITS + cost[p + mid];

      if bits < best_bits {
        best_bits = bits;
        best = Match {
          kind: MatchKind::Long,
          length: mid,
          distance: dist_long[p],
        };
      }

      if long >= 10 {
        let bits = EXTENDED_LONG_BITS + cost[p + long];

        if bits < best_bits {
          best_bits = bits;
          best = Match {
            kind: MatchKind::Long,
            length: long,
            distance: dist_long[p],
          };
        }
      }
    }

    cost[p] = best_bits;
    matches[p] = best;
  }

  // Pass 3: Serialize the selected instructions directly.
  //
  // Each control byte is reserved when its first bit is written.
  // Instruction payload follows the control byte containing its final bit.

  let out = serialize(data, &matches);
  debug_assert_eq!(out.len(), cost[0].div_ceil(8));
  out
}

fn serialize(data: &[u8], matches: &[Match]) -> Vec<u8> {
  let n = data.len();
  let mut out = Vec::with_capacity(n + n / 8 + 3);
  let mut bitpos = 0usize;
  let mut control_index = 0usize;

  let mut emit = |bits: u8, bit_count: u8, payload: &[u8]| {
    for k in 0..bit_count {
      if bitpos.is_multiple_of(8) {
        control_index = out.len();
        out.push(0);
      }

      if (bits >> k) & 1 != 0 {
        out[control_index] |= 1 << (bitpos % 8);
      }

      bitpos += 1;
    }

    out.extend_from_slice(payload);
  };

  let mut pos = 0;

  while pos < n {
    let m = matches[pos];

    match m.kind {
      MatchKind::Literal => {
        emit(1, 1, &data[pos..pos + 1]);
      }

      MatchKind::Short => {
        let size_bits = (m.length - 2) as u8;
        let bits = ((size_bits >> 1) << 2) | ((size_bits & 1) << 3);

        emit(bits, 4, &[(256 - m.distance) as u8]);
      }

      MatchKind::Long => {
        let size_bits = if m.length <= 9 {
          (m.length - 2) as u16
        } else {
          0
        };

        let value = (((8192 - m.distance) as u16) << 3) | size_bits;
        let bytes = value.to_le_bytes();

        if m.length >= 10 {
          emit(
            0b10,
            2,
            &[
              bytes[0],
              bytes[1],
              (m.length - 1) as u8,
            ],
          );
        } else {
          emit(0b10, 2, &bytes);
        }
      }
    }

    pos += m.length;
  }

  // End marker: control bits 0,1 followed by word 0.
  emit(0b10, 2, &[0, 0]);

  out
}

#[cfg(test)]
mod tests {
  use crate::codecs::sega::compression::prs::decompress::prs_decompress;

  use super::*;

  fn assert_roundtrip(compressed: &[u8], data: &[u8]) {
    assert_eq!(prs_decompress(compressed).unwrap(), data);
    let mut decoded = vec![0; data.len()];
    // SAFETY: the checked decoder above validated the stream and its exact
    // decoded size before handing it to prs-rs's unchecked decoder.
    let written =
      unsafe { prs_rs::decomp::prs_decompress_unsafe(compressed, decoded.as_mut_slice()) };
    assert_eq!(written, data.len());
    assert_eq!(decoded, data);
  }

  #[test]
  fn instruction_costs_match_serialization_at_every_control_phase() {
    for (kind, length, control_bits, payload_bytes, bits, expected) in [
      (MatchKind::Short, 2, 4, 1, SHORT_BITS, "4141ff0000"),
      (MatchKind::Short, 5, 4, 1, SHORT_BITS, "5941ff0000"),
      (MatchKind::Long, 3, 2, 2, LONG_BITS, "1541f9ff0000"),
      (MatchKind::Long, 9, 2, 2, LONG_BITS, "1541ffff0000"),
      (
        MatchKind::Long,
        10,
        2,
        3,
        EXTENDED_LONG_BITS,
        "1541f8ff090000",
      ),
      (
        MatchKind::Long,
        256,
        2,
        3,
        EXTENDED_LONG_BITS,
        "1541f8ffff0000",
      ),
    ] {
      // Literal prefixes exercise all eight phases, including instructions
      // whose control bits straddle two control bytes.
      for literals in 1usize..=8 {
        let data = vec![b'A'; literals + length];
        let mut matches = vec![Match::default(); data.len()];
        matches[literals] = Match {
          kind,
          length,
          distance: 1,
        };
        let compressed = serialize(&data, &matches);
        assert_eq!(
          compressed.len(),
          (literals * LITERAL_BITS + bits + END_BITS).div_ceil(8)
        );
        assert_eq!(
          compressed.len(),
          literals + payload_bytes + 2 + (literals + control_bits + 2).div_ceil(8)
        );
        if literals == 1 {
          assert_eq!(hex::encode(&compressed), expected);
        }
        assert_roundtrip(&compressed, &data);
      }
    }
  }

  #[test]
  fn literal_and_end_costs_include_only_final_control_padding() {
    for length in 0usize..=16 {
      let data: Vec<_> = (0..length as u8).collect();
      let compressed = serialize(&data, &vec![Match::default(); length]);
      assert_eq!(compressed.len(), length + 2 + (length + 2).div_ceil(8));
      assert_eq!(
        compressed.len(),
        (length * LITERAL_BITS + END_BITS).div_ceil(8)
      );
      assert_eq!(prs_compress(&data, 256), compressed);
      match length {
        0 => assert_eq!(compressed, [0x02, 0, 0]),
        6 => assert_eq!(
          compressed,
          [
            0xbf,
            0,
            1,
            2,
            3,
            4,
            5,
            0,
            0
          ]
        ),
        7 => assert_eq!(
          compressed,
          [
            0x7f,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            0x01,
            0,
            0
          ]
        ),
        8 => assert_eq!(
          compressed,
          [
            0xff,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            0x02,
            0,
            0
          ]
        ),
        _ => {}
      }
      assert_roundtrip(&compressed, &data);
    }
  }
}
