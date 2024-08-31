
use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use thiserror::Error;
 
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("cw20 error: {0}")]
    Cw20(#[from] cw20_base::ContractError),

    // #[error("Invalid contract config")]
    // InvalidConfig {},

    #[error("Payment error: {0}")]
    Payment(#[from] PaymentError),

    #[error("Invalid deposit")]
    InvalidDeposit {},

    #[error("Invalid concentrated liquidity pool_id {0}")]
    InvalidPoolId(u64),

    #[error("Invalid delegate vault rebalancer address: {0}")]
    InvalidDelegateAddress(String),

    #[error("Invalid vault admin address: {0}")]
    InvalidAdminAddress(String),

    #[error("Contradiction: {reason}")]
    ContradictoryConfig { reason: String },

    #[error("Price factors are String Decimals greater than 1, got: {0}")]
    InvalidPriceFactor(String),

    #[error("Weights are String Decimals in the range [0, 1], got: {0}")]
    InvalidWeight(String)

}

