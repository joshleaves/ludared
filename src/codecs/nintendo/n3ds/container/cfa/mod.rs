//! Nintendo 3DS CFA (CTR File Archive) codec.
//!
//! CFA is the data-only NCCH variant. Header parsing, regions, hash policy and
//! rebuilding are implemented once in `container::ncch::NcchEngine`.

use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::container::ncch::{
  NcchCodecArgs, NcchEngine, metadata::NcchVariant,
};
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/3ds/container/cfa";
const CODEC_NAME: &str = "Nintendo 3DS CFA (data-only NCCH)";
const CODEC_DESC: &str = r#"Nintendo 3DS CFA (data-only NCCH)

Uses the shared NCCH codec engine to decode and rebuild non-executable NCCH images.
The header, ExeFS/RomFS regions, gaps and signatures are preserved under the
shared engine's validation rules. Modified cleartext region hashes are updated;
ciphertext modifications are rejected. RSA signatures are preserved but cannot
be regenerated without Nintendo's private keys.

No arguments.
"#;

pub(crate) struct N3dsCfa;

impl Codec for N3dsCfa {
  fn id(&self) -> &str {
    CODEC_ID
  }
  fn name(&self) -> &str {
    CODEC_NAME
  }
  fn description(&self) -> &str {
    CODEC_DESC
  }
  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    NcchEngine::can_handle(data, Some(NcchVariant::Cfa))
  }
  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("cfa".to_owned())
  }
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    NcchCodecArgs::from_args(args)?;
    NcchEngine::decode(data, args, Some(NcchVariant::Cfa))
  }
  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    NcchCodecArgs::from_args(args)?;
    NcchEngine::encode(artifacts, Some(NcchVariant::Cfa))
  }
}
