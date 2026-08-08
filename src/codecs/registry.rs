use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::snes::cart_exhirom::SnesExHiRom;
use crate::codecs::snes::cart_hirom::SnesHiRom;
use crate::codecs::snes::cart_lorom::SnesLoRom;
use crate::codecs::snes::cart_sa1::SnesSa1Rom;
use crate::codecs::snes::cart_sdd1::SnesSdd1Rom;
use crate::codecs::snes::cart_spc7110::SnesSpc7110Rom;
use log::*;

static BUILTINS: &[&dyn Codec] = &[
  &SnesExHiRom,
  &SnesLoRom,
  &SnesHiRom,
  &SnesSa1Rom,
  &SnesSdd1Rom,
  &SnesSpc7110Rom,
];

pub(crate) struct CodecRegistry;

impl CodecRegistry {
  pub fn builtins() -> &'static [&'static dyn Codec] {
    BUILTINS
  }

  pub fn detect(data: &[u8]) -> Vec<(&dyn Codec, CodecHandlingConfidence)> {
    let mut codecs = BUILTINS
      .iter()
      .map(|codec| {
        debug!("{}", codec.id());
        (*codec, codec.can_handle(data))
      })
      .filter(|(_, confidence)| *confidence != CodecHandlingConfidence::No)
      .collect::<Vec<_>>();
    codecs.sort_by(|(codec_a, confidence_a), (codec_b, confidence_b)| {
      confidence_b
        .cmp(confidence_a) // DESC
        .then_with(|| codec_a.id().cmp(codec_b.id())) // ASC
    });
    codecs
  }
}
