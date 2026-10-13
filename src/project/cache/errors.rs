use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum CacheError {
  #[error("Virtual path is not indexed in the decode cache: {0}")]
  NotIndexed(String),

  #[error("Could not read cached artifact '{0}': {1}")]
  BlobIo(String, std::io::Error),
}
