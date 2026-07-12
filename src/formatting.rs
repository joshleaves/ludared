pub(crate) fn format_size(size: u64) -> String {
  const UNITS: [&str; 5] = [
    "B",
    "KiB",
    "MiB",
    "GiB",
    "TiB",
  ];

  let mut size = size as f64;
  let mut unit = 0;

  while size >= 1024.0 && unit < UNITS.len() - 1 {
    size /= 1024.0;
    unit += 1;
  }

  if unit == 0 {
    format!("{:.0} {}", size, UNITS[unit])
  } else {
    format!("{:.1} {}", size, UNITS[unit])
  }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
  let digits = bytes.to_string();
  let mut out = String::with_capacity(digits.len() + digits.len() / 3);

  for (i, c) in digits.chars().rev().enumerate() {
    if i != 0 && i % 3 == 0 {
      out.push(',');
    }
    out.push(c);
  }

  out.chars().rev().collect::<String>()
}
