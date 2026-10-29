use super::*;

const FORMATS: [Format; 2] = [
  Format::Lz10,
  Format::Lz11,
];

/// Deterministic pseudo-random bytes, so failures are reproducible.
fn noise(length: usize, seed: u32) -> Vec<u8> {
  let mut state = seed;
  (0..length)
    .map(|_| {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      state.to_le_bytes()[0]
    })
    .collect()
}

fn round_trip(format: Format, data: &[u8]) {
  let compressed = compress(data, format).unwrap();
  assert_eq!(compressed[0], format.magic());
  assert_eq!(
    u32::from_le_bytes([
      compressed[1],
      compressed[2],
      compressed[3],
      0
    ]) as usize,
    data.len()
  );
  assert_eq!(
    compressed.len() % 4,
    0,
    "{} streams are padded to four bytes",
    format.label()
  );
  assert_eq!(
    decompress(&compressed, format).unwrap(),
    data,
    "{} round trip failed for {} bytes",
    format.label(),
    data.len()
  );
}

#[test]
fn round_trips_empty() {
  for format in FORMATS {
    round_trip(format, &[]);
  }
}

#[test]
fn round_trips_every_short_length() {
  for format in FORMATS {
    for length in 0..=40 {
      round_trip(format, &noise(length, u32::try_from(length).unwrap() + 1));
    }
  }
}

#[test]
fn round_trips_literals() {
  let data: Vec<u8> = (0..=255).collect();
  for format in FORMATS {
    round_trip(format, &data);
  }
}

#[test]
fn round_trips_repeating_runs() {
  for format in FORMATS {
    round_trip(format, &vec![b'A'; 10_000]);
    round_trip(format, b"ab".repeat(5_000).as_slice());
    round_trip(format, &[0; 257]);
  }
}

#[test]
fn round_trips_matches_of_every_lz11_shape() {
  // A long unique prefix makes the matcher find one long back reference at
  // each of the LZ11 length boundaries (short, 24-bit and 32-bit forms).
  let prefix = noise(5_000, 7);
  for length in [
    3usize,
    16,
    17,
    0x110,
    0x111,
    0x200,
    0x1_000,
  ] {
    let mut data = prefix.clone();
    data.extend_from_slice(&prefix[..length]);
    round_trip(Format::Lz11, &data);
  }
}

#[test]
fn round_trips_distant_matches() {
  // Push a pattern beyond the 4096-byte window, then repeat it.
  let mut data = noise(4_096, 11);
  let tail = noise(64, 12);
  data.extend_from_slice(&tail);
  data.extend_from_slice(&noise(16, 13));
  data.extend_from_slice(&noise(1, 14));
  data.extend_from_slice(&tail);
  for format in FORMATS {
    round_trip(format, &data);
  }
}

#[test]
fn decompresses_a_literal_only_stream() {
  let stream = [
    0x10,
    3,
    0,
    0,
    0b0000_0000,
    b'a',
    b'b',
    b'c',
  ];
  assert_eq!(decompress(&stream, Format::Lz10).unwrap(), b"abc");
}

#[test]
fn decompresses_lz10_references() {
  // "abc" then a six-byte reference three bytes back.
  let stream = [
    0x10,
    9,
    0,
    0,
    0b0001_0000,
    b'a',
    b'b',
    b'c',
    0x30,
    0x02,
  ];
  assert_eq!(decompress(&stream, Format::Lz10).unwrap(), b"abcabcabc");
}

#[test]
fn decompresses_lz11_short_references() {
  // The high nibble stores length - 1, so 5 means six bytes.
  let stream = [
    0x11,
    9,
    0,
    0,
    0b0001_0000,
    b'a',
    b'b',
    b'c',
    0x50,
    0x02,
  ];
  assert_eq!(decompress(&stream, Format::Lz11).unwrap(), b"abcabcabc");
}

#[test]
fn decompresses_lz11_24_bit_references() {
  // One literal, then 19 bytes from one byte back.
  let stream = [
    0x11,
    20,
    0,
    0,
    0b0100_0000,
    b'A',
    0x00,
    0x20,
    0x00,
  ];
  assert_eq!(decompress(&stream, Format::Lz11).unwrap(), vec![b'A'; 20]);
}

#[test]
fn decompresses_lz11_32_bit_references() {
  // One literal, then 300 bytes from one byte back.
  let stream = [
    0x11,
    0x2D,
    0x01,
    0x00,
    0b0100_0000,
    b'B',
    0x10,
    0x01,
    0xB0,
    0x00,
  ];
  assert_eq!(decompress(&stream, Format::Lz11).unwrap(), vec![b'B'; 301]);
}

#[test]
fn decompresses_overlapping_references() {
  // A match may copy bytes it has just produced; "A" then a 7-byte match at
  // displacement 1 must reproduce a run.
  let stream = [
    0x10,
    8,
    0,
    0,
    0b0100_0000,
    b'A',
    0x40,
    0x00,
  ];
  assert_eq!(decompress(&stream, Format::Lz10).unwrap(), vec![b'A'; 8]);
}

#[test]
fn rejects_a_truncated_header() {
  for format in FORMATS {
    let error = decompress(&[0x10, 0, 0], format).unwrap_err();
    assert!(error.to_string().contains("truncated"), "{error}");
  }
}

#[test]
fn rejects_the_wrong_magic() {
  let error = decompress(&[0x11, 0, 0, 0], Format::Lz10).unwrap_err();
  assert!(error.to_string().contains("Not an LZ10"), "{error}");
  let error = decompress(&[0x10, 0, 0, 0], Format::Lz11).unwrap_err();
  assert!(error.to_string().contains("Not an LZ11"), "{error}");
}

#[test]
fn rejects_a_stream_that_ends_mid_token() {
  for format in FORMATS {
    // A literal flag with no literal byte behind it.
    let error = decompress(
      &[
        format.magic(),
        1,
        0,
        0,
        0b1000_0000,
      ],
      format,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ended in the middle"), "{error}");
  }
}

#[test]
fn rejects_a_reference_before_the_output_starts() {
  for format in FORMATS {
    let error = decompress(
      &[
        format.magic(),
        3,
        0,
        0,
        0b1000_0000,
        0x30,
        0x00,
      ],
      format,
    )
    .unwrap_err();
    assert!(
      error.to_string().contains("reaches 1 bytes back"),
      "{error}"
    );
  }
}

#[test]
fn rejects_a_reference_past_the_declared_size() {
  // Three literals followed by a three-byte reference into a five-byte stream.
  let stream = [
    0x10,
    5,
    0,
    0,
    0b0001_0000,
    b'a',
    b'b',
    b'c',
    0x00,
    0x02,
  ];
  let error = decompress(&stream, Format::Lz10).unwrap_err();
  assert!(
    error.to_string().contains("past the declared size"),
    "{error}"
  );
}

#[test]
fn compression_is_deterministic() {
  let data = [
    b"deterministic".repeat(50),
    noise(300, 99),
  ]
  .concat();
  for format in FORMATS {
    let first = compress(&data, format).unwrap();
    let second = compress(&data, format).unwrap();
    assert_eq!(first, second);
  }
}

#[test]
fn compressed_streams_are_padded_with_ff() {
  let compressed = compress(b"x", Format::Lz10).unwrap();
  // Header, one flag byte, one literal, then padding to four bytes.
  assert_eq!(compressed.len(), 8);
  assert_eq!(&compressed[6..], &[0xFF, 0xFF]);
}
