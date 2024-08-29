use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use thiserror::Error;

 
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("cw20 error: {0}")]
    Cw20(#[from] cw20_base::ContractError),

    #[error("Invalid contract config")]
    InvalidConfig {},

    #[error("Payment error: {0}")]
    Payment(#[from] PaymentError),

    #[error("Invalid deposit")]
    InvalidDeposit {},

    #[error("Invalid pool_id: {0}")]
    InvalidPoolId(u64),
}

#[derive(Error, Debug, PartialEq)]
enum ExampleError0 {
    #[error("error 0 happened")]
    Inner 
}


