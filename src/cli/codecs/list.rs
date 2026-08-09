use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;

pub(crate) fn command_codecs_list() -> Result<(), AppError> {
  let mut codecs = CodecRegistry::builtins().iter().collect::<Vec<_>>();
  codecs.sort_by(|a, b| a.id().cmp(b.id()));
  for codec in codecs {
    println!("{}: {}", codec.id(), codec.name());
  }

  Ok(())
}
