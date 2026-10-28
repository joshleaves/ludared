//! Tests for the Nintendo 3DS BCFNT codec.

use super::metadata::{
  BcfntMetadata, ByteOrder, CmapMetadata, CwdhMetadata, TextureFormat, decode_hex,
};
use super::texture::{decode_sheet, encode_sheet, png_to_sheet, sheet_size, sheet_to_png};
use super::*;

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt.bcfnt"
));

const ETC1_SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt_etc1.bcfnt"
));

const PADDED_SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt_padded.bcfnt"
));

fn decode() -> Vec<DecodedArtifact> {
  N3dsBcfnt
    .decode(SOURCE, None)
    .expect("the committed BCFNT fixture should decode")
}

fn artifact<'a>(artifacts: &'a [DecodedArtifact], name: &str) -> &'a DecodedArtifact {
  artifacts
    .iter()
    .find(|artifact| artifact.name == name)
    .unwrap_or_else(|| panic!("expected a {name} artifact"))
}

fn artifact_mut<'a>(artifacts: &'a mut [DecodedArtifact], name: &str) -> &'a mut DecodedArtifact {
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == name)
    .unwrap_or_else(|| panic!("expected a {name} artifact"))
}

fn metadata(artifacts: &[DecodedArtifact]) -> BcfntMetadata {
  BcfntMetadata::from_json(&artifact(artifacts, metadata::METADATA_NAME).data)
    .expect("BCFNT metadata should parse")
}

#[test]
fn detects_the_fixture_as_a_bcfnt_font() {
  assert_eq!(
    N3dsBcfnt.can_handle(SOURCE).unwrap(),
    CodecHandlingConfidence::Certain
  );
  assert_eq!(
    N3dsBcfnt.can_handle(b"not a font").unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn rebuilds_the_committed_fixture_byte_for_byte() {
  let artifacts = decode();
  let rebuilt = N3dsBcfnt
    .encode(Vec::new(), &artifacts, None)
    .expect("the fixture should encode");

  assert_eq!(rebuilt, SOURCE);
}

#[test]
fn exposes_the_font_sections_and_texture() {
  let artifacts = decode();
  let names: Vec<&str> = artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect();

  assert_eq!(
    names,
    [
      metadata::METADATA_NAME,
      metadata::CWDH_NAME,
      metadata::CMAP_NAME,
      "texture0.png",
    ]
  );

  let metadata = metadata(&artifacts);
  assert_eq!(metadata.magic, FontMagic::Cfnt);
  assert_eq!(metadata.byte_order, ByteOrder::Little);
  assert_eq!(metadata.version, VERSION);
  assert_eq!(metadata.font.height, 24);
  assert_eq!(metadata.font.width, 24);
  assert_eq!(metadata.font.ascent, 20);
  assert_eq!(metadata.texture.format, TextureFormat::Rgba8);
  assert_eq!(metadata.texture.width, 8);
  assert_eq!(metadata.texture.height, 8);
  assert_eq!(sheet_size(metadata.texture.format, 8, 8).unwrap(), 256);

  let cwdh = CwdhMetadata::from_json(&artifact(&artifacts, metadata::CWDH_NAME).data).unwrap();
  assert_eq!(cwdh.sections.len(), 1);
  assert_eq!(cwdh.sections[0].start, 0);
  assert_eq!(cwdh.sections[0].end, 2);
  assert_eq!(cwdh.sections[0].entries.len(), 3);
  assert_eq!(cwdh.sections[0].entries[1].char_width, 6);

  let cmap = CmapMetadata::from_json(&artifact(&artifacts, metadata::CMAP_NAME).data).unwrap();
  match &cmap.sections[0] {
    CmapSection::Scan {
      start,
      end,
      unknown,
      entries,
      ..
    } => {
      assert_eq!((*start, *end), (0x20, 0x22));
      assert_eq!(*unknown, 0x1234);
      assert_eq!(
        entries[1],
        CmapEntry {
          code: 0x21,
          glyph: 1
        }
      );
    }
    other => panic!("expected a scan CMAP, found {other:?}"),
  }

  // Sheets begin directly after the fixed headers: the TGLP header ends at
  // 0x54, so a canonical font needs no gap bytes.
  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(
    read_u32(&rebuilt, 0x34 + 0x1C, ByteOrder::Little).unwrap(),
    0x54
  );
  assert_eq!(
    read_u32(&rebuilt, 0x34 + 0x04, ByteOrder::Little).unwrap(),
    0x20 + 256
  );
}

#[test]
fn changes_a_character_width_and_a_mapping() {
  let mut artifacts = decode();

  {
    let artifact = artifact_mut(&mut artifacts, metadata::CWDH_NAME);
    let mut cwdh = CwdhMetadata::from_json(&artifact.data).unwrap();
    cwdh.sections[0].entries[1].char_width = 42;
    artifact.data = cwdh.to_json().unwrap();
  }

  {
    let artifact = artifact_mut(&mut artifacts, metadata::CMAP_NAME);
    let mut cmap = CmapMetadata::from_json(&artifact.data).unwrap();
    match &mut cmap.sections[0] {
      CmapSection::Scan { entries, .. } => entries[1].glyph = 2,
      other => panic!("expected a scan CMAP, found {other:?}"),
    }
    artifact.data = cmap.to_json().unwrap();
  }

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let cwdh = CwdhMetadata::from_json(&artifact(&decoded, metadata::CWDH_NAME).data).unwrap();
  let cmap = CmapMetadata::from_json(&artifact(&decoded, metadata::CMAP_NAME).data).unwrap();

  assert_eq!(cwdh.sections[0].entries[1].char_width, 42);
  match &cmap.sections[0] {
    CmapSection::Scan { entries, .. } => assert_eq!(entries[1].glyph, 2),
    other => panic!("expected a scan CMAP, found {other:?}"),
  }
}

#[test]
fn rebuilds_a_changed_texture_png() {
  let mut artifacts = decode();
  let mut sheet = {
    let png = artifact(&artifacts, "texture0.png");
    png_to_sheet(&png.data, 8, 8).unwrap()
  };
  sheet.pixels[0] = [1, 2, 3, 4];

  let png = sheet_to_png(&sheet).unwrap();
  artifact_mut(&mut artifacts, "texture0.png").data = png;

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let metadata = metadata(&decoded);
  let png = &artifact(&decoded, "texture0.png").data;
  let sheet = png_to_sheet(png, metadata.texture.width, metadata.texture.height).unwrap();

  assert_eq!(sheet.pixels[0], [1, 2, 3, 4]);
}

#[test]
fn round_trips_a_big_endian_rebuild() {
  let mut artifacts = decode();
  {
    let artifact = artifact_mut(&mut artifacts, metadata::METADATA_NAME);
    let mut metadata = BcfntMetadata::from_json(&artifact.data).unwrap();
    metadata.byte_order = ByteOrder::Big;
    artifact.data = metadata.to_json().unwrap();
  }

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(&rebuilt[4..6], &[0xFE, 0xFF]);

  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  assert_eq!(metadata(&decoded).byte_order, ByteOrder::Big);

  let again = N3dsBcfnt.encode(Vec::new(), &decoded, None).unwrap();
  assert_eq!(again, rebuilt);
}

#[test]
fn rejects_missing_and_unexpected_artifacts() {
  let mut artifacts = decode();
  let png = artifacts
    .iter()
    .position(|artifact| artifact.name == "texture0.png")
    .map(|index| artifacts.remove(index))
    .unwrap();
  artifacts.push(DecodedArtifact {
    name: "texture1.png".to_owned(),
    data: png.data,
  });
  let error = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("texture0.png"), "{error}");

  let mut artifacts = decode();
  artifacts.push(DecodedArtifact {
    name: "surprise.bin".to_owned(),
    data: Vec::new(),
  });
  let error = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("Unexpected artifact"), "{error}");
}

#[test]
fn preserves_a_padded_tglp_layout_byte_for_byte() {
  let padded = N3dsBcfnt.decode(PADDED_SOURCE, None).unwrap();
  let canonical = decode();

  assert_eq!(padded.len(), canonical.len());
  for (padded, canonical) in padded.iter().zip(&canonical) {
    assert_eq!(padded.name, canonical.name);
    if padded.name != metadata::METADATA_NAME {
      assert_eq!(padded.data, canonical.data, "{}", padded.name);
    }
  }

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &padded, None).unwrap();
  assert_eq!(rebuilt, PADDED_SOURCE);
}

#[test]
fn keeps_multiple_sheets_contiguous_and_relocates_later_sections() {
  let mut artifacts = decode();
  let png = artifact(&artifacts, "texture0.png").data.clone();
  artifacts.push(DecodedArtifact {
    name: "texture1.png".to_owned(),
    data: png,
  });

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_u32(&rebuilt, 0x34 + 0x10, ByteOrder::Little).unwrap(),
    2
  );
  assert_eq!(
    read_u32(&rebuilt, 0x34 + 0x1C, ByteOrder::Little).unwrap(),
    0x54
  );
  assert_eq!(
    read_u32(&rebuilt, 0x34 + 0x04, ByteOrder::Little).unwrap(),
    0x20 + 2 * 256
  );
  // CWDH starts after both sheets, and FINF points at it.
  assert_eq!(
    read_u32(&rebuilt, 0x14 + 0x14, ByteOrder::Little).unwrap(),
    0x254 + 8
  );

  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let names: Vec<&str> = decoded
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect();
  assert_eq!(
    names,
    [
      metadata::METADATA_NAME,
      metadata::CWDH_NAME,
      metadata::CMAP_NAME,
      "texture0.png",
      "texture1.png",
    ]
  );
  assert_eq!(
    artifact(&decoded, "texture0.png").data,
    artifact(&decoded, "texture1.png").data
  );
  assert_eq!(
    artifact(&decoded, "texture0.png").data,
    artifact(&artifacts, "texture0.png").data
  );
}

#[test]
fn preserves_tglp_gap_and_file_trailer_bytes() {
  let mut source = PADDED_SOURCE.to_vec();
  let tglp_start = 0x34;
  let sheet_data_offset = read_u32(&source, tglp_start + 0x1C, ByteOrder::Little).unwrap() as usize;
  let tglp_end =
    read_u32(&source, tglp_start + 4, ByteOrder::Little).unwrap() as usize + tglp_start;
  // Include non-zero bytes both before the sheet data and after the sections.
  source[0x54..sheet_data_offset].copy_from_slice(&[0xA5; 0x2C]);
  source.extend_from_slice(&[
    0xDE,
    0xAD,
    0xBE,
    0xEF,
  ]);
  let file_size = source.len() as u32;
  source[0x0C..0x10].copy_from_slice(&file_size.to_le_bytes());

  let artifacts = N3dsBcfnt.decode(&source, None).unwrap();
  let metadata = metadata(&artifacts);
  let expected_gap = "a5".repeat(0x2C);
  assert_eq!(metadata.tglp_gap.as_deref(), Some(expected_gap.as_str()));
  assert_eq!(metadata.file_trailer.as_deref(), Some("deadbeef"));

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt, source);
  assert_eq!(&rebuilt[0x54..sheet_data_offset], &[0xA5; 0x2C]);

  assert!(rebuilt.ends_with(&[
    0xDE,
    0xAD,
    0xBE,
    0xEF
  ]));
  assert!(tglp_end < rebuilt.len() - 4);
}

#[test]
fn preserves_bytes_after_the_texture_sheets_inside_tglp() {
  let mut source = PADDED_SOURCE.to_vec();
  let tglp_start = 0x34;
  let sheet_data_offset = read_u32(&source, tglp_start + 0x1C, ByteOrder::Little).unwrap() as usize;
  let sheets_end = sheet_data_offset + 256;
  let old_tglp_end =
    read_u32(&source, tglp_start + 4, ByteOrder::Little).unwrap() as usize + tglp_start;
  source.splice(
    sheets_end..sheets_end,
    [
      0xCA,
      0xFE,
      0xBA,
      0xBE,
    ],
  );
  let new_tglp_end = old_tglp_end + 4;
  source[tglp_start + 4..tglp_start + 8]
    .copy_from_slice(&((new_tglp_end - tglp_start) as u32).to_le_bytes());
  for offset in [
    0x14 + 0x14,
    0x14 + 0x18,
  ] {
    let value = read_u32(&source, offset, ByteOrder::Little).unwrap() + 4;
    source[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
  }
  let file_size = source.len() as u32;
  source[0x0C..0x10].copy_from_slice(&file_size.to_le_bytes());

  let artifacts = N3dsBcfnt.decode(&source, None).unwrap();
  let metadata = metadata(&artifacts);
  assert_eq!(metadata.tglp_trailing.as_deref(), Some("cafebabe"));
  assert_eq!(
    N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap(),
    source
  );
}

#[test]
fn enforces_the_file_size_limit_without_allocating_the_oversized_input() {
  assert!(validate_file_size(MAX_FILE_SIZE).is_ok());
  let error = validate_file_size(MAX_FILE_SIZE + 1).unwrap_err();
  assert!(
    error.to_string().contains("maximum supported size"),
    "{error}"
  );
}

#[test]
fn rejects_texture_dimensions_that_exceed_the_expanded_image_limit() {
  let mut source = SOURCE.to_vec();
  let tglp_start = 0x34;
  source[tglp_start + 0x18..tglp_start + 0x1A].copy_from_slice(&u16::MAX.to_le_bytes());
  source[tglp_start + 0x1A..tglp_start + 0x1C].copy_from_slice(&u16::MAX.to_le_bytes());

  let error = match N3dsBcfnt.decode(&source, None) {
    Ok(_) => panic!("oversized texture dimensions should be rejected"),
    Err(error) => error,
  };
  assert!(error.to_string().contains("expand to"), "{error}");
  assert_ne!(
    N3dsBcfnt.can_handle(&source).unwrap(),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn rejects_malformed_section_chains_during_detection() {
  let mut damaged_cwdh = SOURCE.to_vec();
  // FINF CWDH pointer targets a missing section header, despite otherwise valid leading structures.
  damaged_cwdh[0x14 + 0x14..0x14 + 0x18].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
  assert_ne!(
    N3dsBcfnt.can_handle(&damaged_cwdh).unwrap(),
    CodecHandlingConfidence::Certain
  );

  let mut damaged_cmap = SOURCE.to_vec();
  damaged_cmap[0x14 + 0x18..0x14 + 0x1C].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
  assert_ne!(
    N3dsBcfnt.can_handle(&damaged_cmap).unwrap(),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn round_trips_a_big_endian_cfnu_file() {
  let mut source = PADDED_SOURCE.to_vec();
  source[..4].copy_from_slice(b"CFNU");
  source[4..6].copy_from_slice(&[0xFE, 0xFF]);
  source[6..8].reverse();
  // Swap each multibyte integer in the fixed headers.
  for offset in [
    8,
    0x0C,
    0x10,
    0x18,
    0x24,
    0x28,
    0x2C,
    0x38,
    0x40,
    0x50,
  ] {
    source[offset..offset + 4].reverse();
  }
  for offset in [
    0x1E,
    0x44,
    0x46,
    0x48,
    0x4A,
    0x4C,
    0x4E,
  ] {
    source[offset..offset + 2].reverse();
  }
  let cwdh_start = 0x80 + 256;
  let cmap_start = cwdh_start + 0x19;
  source[cwdh_start + 4..cwdh_start + 8].reverse();
  source[cwdh_start + 8..cwdh_start + 10].reverse();
  source[cwdh_start + 10..cwdh_start + 12].reverse();
  source[cwdh_start + 12..cwdh_start + 16].reverse();
  source[cmap_start + 4..cmap_start + 8].reverse();
  for offset in [8, 10, 12, 14] {
    source[cmap_start + offset..cmap_start + offset + 2].reverse();
  }
  source[cmap_start + 16..cmap_start + 20].reverse();
  for offset in (cmap_start + 0x14..cmap_start + 0x22).step_by(2) {
    source[offset..offset + 2].reverse();
  }
  let artifacts = N3dsBcfnt.decode(&source, None).unwrap();
  assert_eq!(metadata(&artifacts).magic, FontMagic::Cfnu);
  assert_eq!(metadata(&artifacts).byte_order, ByteOrder::Big);
  assert_eq!(
    N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap(),
    source
  );
}

#[test]
fn rejects_a_png_with_the_wrong_dimensions() {
  let mut artifacts = decode();
  {
    let png = artifact(&artifacts, "texture0.png");
    let sheet = png_to_sheet(&png.data, 8, 8).unwrap();
    let smaller = texture::Sheet {
      width: 4,
      height: 4,
      pixels: sheet.pixels[..16].to_vec(),
    };
    artifact_mut(&mut artifacts, "texture0.png").data = sheet_to_png(&smaller).unwrap();
  }

  let error = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("4x4"), "{error}");
}

#[test]
fn rejects_a_raw_copy_alongside_a_png() {
  let mut artifacts = decode();
  artifacts.push(DecodedArtifact {
    name: "texture0.bin".to_owned(),
    data: vec![0; 256],
  });

  let error = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("Duplicate artifact"), "{error}");
}

#[test]
fn refuses_to_encode_hilo8_from_pixels() {
  let sheet = texture::Sheet {
    width: 8,
    height: 8,
    pixels: vec![[1, 2, 3, 4]; 64],
  };

  let error = encode_sheet(&sheet, TextureFormat::Hilo8, ByteOrder::Little).unwrap_err();
  assert!(error.to_string().contains("HILO8"), "{error}");
}

#[test]
fn round_trips_each_linear_texture_format() {
  let cases = [
    (TextureFormat::Rgba8, [1, 2, 3, 4], [1, 2, 3, 4]),
    (TextureFormat::Rgb8, [10, 20, 30, 255], [10, 20, 30, 255]),
    (
      TextureFormat::Rgba5551,
      [0, 0x84, 0xFF, 0xFF],
      [0, 0x84, 0xFF, 0xFF],
    ),
    (
      TextureFormat::Rgb565,
      [0, 0x82, 0xFF, 255],
      [0, 0x82, 0xFF, 255],
    ),
    (
      TextureFormat::Rgba4,
      [0, 0x88, 0xFF, 0xAA],
      [0, 0x88, 0xFF, 0xAA],
    ),
    (
      TextureFormat::La8,
      [100, 100, 100, 200],
      [100, 100, 100, 200],
    ),
    (
      TextureFormat::L8,
      [100, 100, 100, 255],
      [100, 100, 100, 255],
    ),
    (TextureFormat::A8, [255, 255, 255, 77], [255, 255, 255, 77]),
    (
      TextureFormat::La4,
      [
        0x88,
        0x88,
        0x88,
        0xAA,
      ],
      [
        0x88,
        0x88,
        0x88,
        0xAA,
      ],
    ),
    (
      TextureFormat::L4,
      [
        0x88,
        0x88,
        0x88,
        255,
      ],
      [
        0x88,
        0x88,
        0x88,
        255,
      ],
    ),
    (
      TextureFormat::A4,
      [255, 255, 255, 0xAA],
      [255, 255, 255, 0xAA],
    ),
  ];

  for (format, pixel, expected) in cases {
    let sheet = texture::Sheet {
      width: 8,
      height: 8,
      pixels: vec![pixel; 64],
    };
    let bytes = encode_sheet(&sheet, format, ByteOrder::Little).unwrap();
    let decoded = decode_sheet(&bytes, 8, 8, format, ByteOrder::Little).unwrap();

    assert_eq!(decoded.pixels, vec![expected; 64], "{format:?}");
  }
}

#[test]
fn round_trips_every_grayscale_value_through_la8_and_l8() {
  for value in 0..=u8::MAX {
    let alpha = value.wrapping_mul(37).wrapping_add(11);
    let sheet = texture::Sheet {
      width: 8,
      height: 8,
      pixels: vec![
        [
          value,
          value,
          value,
          alpha
        ];
        64
      ],
    };

    let la8 = encode_sheet(&sheet, TextureFormat::La8, ByteOrder::Little).unwrap();
    assert_eq!(la8[0], value, "LA8 luminance for gray {value}");
    assert_eq!(la8[1], alpha, "LA8 alpha for gray {value}");

    let decoded = decode_sheet(&la8, 8, 8, TextureFormat::La8, ByteOrder::Little).unwrap();
    assert_eq!(
      decoded.pixels,
      vec![
        [
          value,
          value,
          value,
          alpha
        ];
        64
      ],
      "LA8 round trip for gray {value}"
    );

    let l8 = encode_sheet(&sheet, TextureFormat::L8, ByteOrder::Little).unwrap();
    assert_eq!(l8[0], value, "L8 luminance for gray {value}");

    let decoded = decode_sheet(&l8, 8, 8, TextureFormat::L8, ByteOrder::Little).unwrap();
    assert_eq!(
      decoded.pixels,
      vec![
        [
          value,
          value,
          value,
          0xFF
        ];
        64
      ],
      "L8 round trip for gray {value}"
    );
  }
}

#[test]
fn rounds_non_grayscale_luminance_to_the_nearest_integer() {
  let cases = [
    ([255, 0, 0, 17], 54),
    ([0, 255, 0, 17], 182),
    ([0, 0, 255, 17], 18),
    ([10, 20, 30, 17], 19),
    ([1, 2, 3, 17], 2),
    ([255, 255, 255, 17], 255),
  ];

  for (pixel, expected) in cases {
    let sheet = texture::Sheet {
      width: 8,
      height: 8,
      pixels: vec![pixel; 64],
    };
    let bytes = encode_sheet(&sheet, TextureFormat::La8, ByteOrder::Little).unwrap();

    assert_eq!(bytes[0], expected, "LA8 luminance for {pixel:?}");
    assert_eq!(bytes[1], pixel[3], "LA8 alpha for {pixel:?}");

    let decoded = decode_sheet(&bytes, 8, 8, TextureFormat::La8, ByteOrder::Little).unwrap();
    assert_eq!(
      decoded.pixels,
      vec![
        [
          expected,
          expected,
          expected,
          pixel[3]
        ];
        64
      ],
      "LA8 round trip for {pixel:?}"
    );
  }
}

#[test]
fn reencoding_a_decoded_la8_sheet_is_deterministic() {
  let sheet = texture::Sheet {
    width: 8,
    height: 8,
    pixels: (0..64)
      .map(|index| {
        let value = index as u8;
        [
          value,
          value.wrapping_mul(3),
          value.wrapping_mul(7),
          value.wrapping_mul(11),
        ]
      })
      .collect(),
  };

  let first = encode_sheet(&sheet, TextureFormat::La8, ByteOrder::Little).unwrap();
  let second = encode_sheet(&sheet, TextureFormat::La8, ByteOrder::Little).unwrap();
  assert_eq!(first, second, "LA8 encoding should be deterministic");

  let decoded = decode_sheet(&first, 8, 8, TextureFormat::La8, ByteOrder::Little).unwrap();
  let reencoded = encode_sheet(&decoded, TextureFormat::La8, ByteOrder::Little).unwrap();
  assert_eq!(
    reencoded, first,
    "LA8 should survive a decode/encode round trip"
  );
}

#[test]
fn decodes_etc1_and_etc1a4_blocks() {
  let etc1 = vec![0u8; 32];
  let sheet = decode_sheet(&etc1, 8, 8, TextureFormat::Etc1, ByteOrder::Little).unwrap();
  assert_eq!(sheet.pixels, vec![[2, 2, 2, 255]; 64]);

  let mut etc1a4 = vec![0xFF; 64];
  for block in etc1a4.as_chunks_mut::<16>().0 {
    block[0..8].fill(0xFF);
    block[8..16].fill(0);
  }
  let sheet = decode_sheet(&etc1a4, 8, 8, TextureFormat::Etc1A4, ByteOrder::Little).unwrap();
  assert_eq!(sheet.pixels, vec![[2, 2, 2, 255]; 64]);
}

#[test]
fn encodes_etc1_and_etc1a4_from_pixels() {
  let sheet = texture::Sheet {
    width: 8,
    height: 8,
    pixels: (0..64)
      .map(|index| {
        let x = index % 8;
        let y = index / 8;
        [
          (x * 32) as u8,
          (y * 32) as u8,
          ((x + y) * 16) as u8,
          (x * 16 + y * 8) as u8,
        ]
      })
      .collect(),
  };

  for format in [
    TextureFormat::Etc1,
    TextureFormat::Etc1A4,
  ] {
    for order in [
      ByteOrder::Little,
      ByteOrder::Big,
    ] {
      let bytes = encode_sheet(&sheet, format, order).unwrap();
      assert_eq!(
        bytes.len(),
        sheet_size(format, 8, 8).unwrap(),
        "{format:?} {order:?}"
      );

      let again = encode_sheet(&sheet, format, order).unwrap();
      assert_eq!(
        bytes, again,
        "{format:?} {order:?} encoding must be deterministic"
      );

      let decoded = decode_sheet(&bytes, 8, 8, format, order).unwrap();
      for (source, decoded) in sheet.pixels.iter().zip(&decoded.pixels) {
        for channel in 0..3 {
          assert!(
            source[channel].abs_diff(decoded[channel]) <= 48,
            "{format:?} {order:?} channel {channel} was {} and decoded to {}",
            source[channel],
            decoded[channel]
          );
        }

        if format == TextureFormat::Etc1A4 {
          assert!(
            source[3].abs_diff(decoded[3]) <= 8,
            "ETC1A4 {order:?} alpha was {} and decoded to {}",
            source[3],
            decoded[3]
          );
        } else {
          assert_eq!(decoded[3], 255);
        }
      }
    }
  }
}

#[test]
fn rebuilds_an_etc1_font_from_its_png_artifact() {
  let artifacts = N3dsBcfnt.decode(ETC1_SOURCE, None).unwrap();
  let names: Vec<&str> = artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect();
  assert_eq!(
    names,
    [
      metadata::METADATA_NAME,
      metadata::CWDH_NAME,
      metadata::CMAP_NAME,
      "texture0.png",
    ]
  );
  assert_eq!(metadata(&artifacts).texture.format, TextureFormat::Etc1);

  let before = png_to_sheet(&artifact(&artifacts, "texture0.png").data, 8, 8).unwrap();
  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let after = png_to_sheet(&artifact(&decoded, "texture0.png").data, 8, 8).unwrap();

  for (before, after) in before.pixels.iter().zip(&after.pixels) {
    for channel in 0..3 {
      assert!(
        before[channel].abs_diff(after[channel]) <= 48,
        "ETC1 recompression changed channel {channel} from {} to {}",
        before[channel],
        after[channel]
      );
    }
    assert_eq!(after[3], 255);
  }
}

#[test]
fn round_trips_direct_and_table_cmaps() {
  let mut artifacts = decode();
  let cmap = CmapMetadata {
    sections: vec![
      CmapSection::Direct {
        start: 0x20,
        end: 0x22,
        unknown: 1,
        index_offset: 5,
        padding: String::new(),
      },
      CmapSection::Table {
        start: 0x30,
        end: 0x32,
        unknown: 2,
        index_table: vec![0, 1, 0xFFFF],
        padding: "cc".to_owned(),
      },
    ],
  };
  artifact_mut(&mut artifacts, metadata::CMAP_NAME).data = cmap.to_json().unwrap();

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let cmap = CmapMetadata::from_json(&artifact(&decoded, metadata::CMAP_NAME).data).unwrap();

  assert_eq!(cmap.sections.len(), 2);
  match &cmap.sections[0] {
    CmapSection::Direct {
      start,
      end,
      unknown,
      index_offset,
      ..
    } => {
      assert_eq!((*start, *end, *unknown, *index_offset), (0x20, 0x22, 1, 5));
    }
    other => panic!("expected a direct CMAP, found {other:?}"),
  }
  match &cmap.sections[1] {
    CmapSection::Table {
      start,
      end,
      unknown,
      index_table,
      padding,
    } => {
      assert_eq!((*start, *end, *unknown), (0x30, 0x32, 2));
      assert_eq!(index_table, &[0, 1, 0xFFFF]);
      assert_eq!(padding, "cc");
    }
    other => panic!("expected a table CMAP, found {other:?}"),
  }
}

#[test]
fn preserves_the_cmap_padding_and_unknown_bytes() {
  let mut artifacts = decode();
  {
    let artifact = artifact_mut(&mut artifacts, metadata::CMAP_NAME);
    let mut cmap = CmapMetadata::from_json(&artifact.data).unwrap();
    let section = cmap.sections.remove(0);
    let section = with_cmap_padding(section, "aabb".to_owned());
    cmap.sections.push(section);
    artifact.data = cmap.to_json().unwrap();
  }

  let rebuilt = N3dsBcfnt.encode(Vec::new(), &artifacts, None).unwrap();
  let decoded = N3dsBcfnt.decode(&rebuilt, None).unwrap();
  let cmap = CmapMetadata::from_json(&artifact(&decoded, metadata::CMAP_NAME).data).unwrap();

  assert_eq!(cmap.sections[0].padding(), "aabb");
  assert_eq!(cmap.sections[0].start(), 0x20);
  assert_eq!(
    decode_hex("padding", cmap.sections[0].padding()).unwrap(),
    [0xAA, 0xBB]
  );
}
