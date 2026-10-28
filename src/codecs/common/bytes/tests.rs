use super::*;

#[test]
fn reads_fields_at_offsets() {
  let bytes = [
    0xAA,
    0x34,
    0x12,
    0x78,
    0x56,
    0x34,
    0x12,
    0xEF,
    0xCD,
    0xAB,
    0x89,
  ];

  assert_eq!(read_field::<2>(&bytes, 1).unwrap(), [0x34, 0x12]);
  assert_eq!(read_u16_le(&bytes, 1).unwrap(), 0x1234);
  assert_eq!(read_u16_be(&bytes, 1).unwrap(), 0x3412);
  assert_eq!(read_u32_le(&bytes, 3).unwrap(), 0x1234_5678);
  assert_eq!(read_u32_be(&bytes, 3).unwrap(), 0x7856_3412);
  assert_eq!(read_u64_le(&bytes, 3).unwrap(), 0x89AB_CDEF_1234_5678);
  assert_eq!(read_u64_be(&bytes, 3).unwrap(), 0x7856_3412_EFCD_AB89);
}

#[test]
fn rejects_fields_that_do_not_fit() {
  let bytes = [0x12, 0x34, 0x56];

  for error in [
    read_field::<4>(&bytes, 0).unwrap_err(),
    read_u16_le(&bytes, 2).unwrap_err(),
    read_u16_be(&bytes, 2).unwrap_err(),
    read_u32_le(&bytes, 0).unwrap_err(),
    read_u32_be(&bytes, 0).unwrap_err(),
    read_u64_le(&bytes, 0).unwrap_err(),
    read_u64_be(&bytes, 0).unwrap_err(),
  ] {
    let message = error.to_string();
    assert!(message.contains("truncated"), "{message}");
    assert!(message.contains("field at"), "{message}");
  }
}

#[test]
fn rejects_an_offset_past_the_end() {
  let error = read_field::<1>(&[0], 1).unwrap_err().to_string();
  assert!(error.contains("at 0x1"), "{error}");
}
