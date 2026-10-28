use serde::Deserialize;

pub(crate) mod iso9660;
pub(crate) mod udf;

#[derive(PartialEq, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CdRomFileSystem {
  #[default]
  Raw,
  Iso9660,
}
