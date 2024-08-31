use core::fmt;

use cosmwasm_std::{ConversionOverflowError, DivideByZeroError, OverflowError, StdError};
use cw_utils::PaymentError;
use osmosis_std::types::osmosis::concentratedliquidity::v1beta1::FullPositionBreakdown;
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

