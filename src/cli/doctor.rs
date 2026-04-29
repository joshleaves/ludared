use crate::{app_error::AppError, configuration::Configuration};

pub(crate) fn command_doctor() -> Result<(), AppError> {
  let config = Configuration::load_default()?;
  println!("{:?}", config);

  Ok(())
}
