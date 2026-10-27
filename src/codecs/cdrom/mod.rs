use serde::Deserialize;

mod common_disk;
mod common_mode;
mod edc_ecc;
pub(crate) mod iso9660;
pub(crate) mod mode1;
pub(crate) mod udf;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CdRomFileSystem {
  #[default]
  Raw,
  Iso9660,
}
