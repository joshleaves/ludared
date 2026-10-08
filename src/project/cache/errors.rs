use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum CacheError {
  #[error("Virtual path is not indexed in the decode cache: {0}")]
  NotIndexed(String),
}
