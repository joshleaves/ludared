use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use serde::Deserialize;

const CODEC_ID: &str = "std/generic/extract_bytes";
const CODEC_NAME: &str = "Extract bytes";
const CODEC_DESC: &str = "Raw bytes extractor

Extracts a range of bytes from the input into a separate artifact.

Available arguments:
  target
    String, output artifact name.
  offset
    Numeric, byte offset at which extraction starts.
  length
    Numeric, number of bytes to extract.
";

pub(crate) struct ExtractBytes;

#[derive(Debug, Deserialize)]
pub(crate) struct ExtractBytesCodecArgs {
  pub(crate) target: String,
  pub(crate) offset: usize,
  pub(crate) length: usize,
}

impl LudaredCodecArgs for ExtractBytesCodecArgs {}

impl Codec for ExtractBytes {
  fn id(&self) -> &'static str {
    CODEC_ID
  }
  fn name(&self) -> &'static str {
    CODEC_NAME
  }
  fn description(&self) -> &'static str {
    CODEC_DESC
  }

  fn can_handle(&self, _: &[u8]) -> CodecHandlingConfidence {
    CodecHandlingConfidence::Certain
  }

  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError> {
    let args = ExtractBytesCodecArgs::from_args(args)?;
    Ok(match args.target.rsplit_once('.') {
      Some((name, _)) => name.to_owned(),
      None => args.target,
    })
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let args = ExtractBytesCodecArgs::from_args(args)?;
    let from = args.offset;
    let to = args.offset + args.length;
    if to > data.len() {
      return Err(CodecError::Message(format!(
        "Out of bounds extraction: {}-{} (Maximum: {})",
        from,
        to,
        data.len()
      )));
    }
    Ok(vec![DecodedArtifact {
      name: args.target,
      data: data[from..to].to_vec(),
    }])
  }
}
