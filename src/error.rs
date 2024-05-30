use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use thiserror::Error;

 
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Invalid contract config")]
    InvalidConfig {},

    #[error("Payment error: {0}")]
    Payment(#[from] PaymentError)
}
