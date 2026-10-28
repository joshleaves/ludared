use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
  #[error("Virtual path is not indexed in the decode cache: {0}. Run 'unpack' command")]
  NotIndexed(String),

  #[error("Could not access cached artifact '{0}': {1}")]
  BlobIo(String, std::io::Error),
}
