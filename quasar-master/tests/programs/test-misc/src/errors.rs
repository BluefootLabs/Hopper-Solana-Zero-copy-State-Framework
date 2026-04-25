use quasar_lang::prelude::*;

#[error_code]
pub enum TestError {
    Unauthorized,
    InvalidAddress,
    CustomConstraint,
}
