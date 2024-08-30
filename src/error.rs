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

    // Refactored error types TODO FIXME the above ones
    #[error("Invalid deposit: {0}")]
    Deposit(#[from] DepositError)
}

#[derive(Error, Debug, PartialEq)]
pub enum DepositError {
    #[error("{0}")]
    InvalidProportion(#[from] InvalidProportionError),

    // #[error("")]
}

#[derive(Error, Debug, PartialEq)]
pub enum PositionBalanceComputationError {
    #[error("Failed to query position {0}, it might not exist")]
    NonExistentPosition(u64), // Impossible!

    #[error("test")]
    Uint128FromStringConversionError(#[from] StdError), // Impossible!

    #[error("Unexpected overflow: {0}")]
    Overflow(#[from] OverflowError), // Impossible!
}

#[derive(Error, Debug, PartialEq)]
pub enum SharesAndAmountsComputationError {
    #[error("{0}")]
    TokenSupplyQueryError(#[from] StdError),

    #[error("hi")]
    TotalContractBalancesComputation(#[from] TotalContractBalancesComputationError),

    #[error("Unexpected overflow: {0}")]
    Overflow(#[from] OverflowError),

    #[error("Unexpected division by 0: {0}")]
    DivideByZero(#[from] DivideByZeroError),

    #[error("Unexpected conversion overflow error: {0}")]
    ConversionOverflow(#[from] ConversionOverflowError),

    #[error("{0}")]
    InvalidProportion(InvalidProportionError)
}

#[derive(Error, Debug, PartialEq)]
pub enum TotalContractBalancesComputationError {
    #[error("{0}")]
    Std(#[from] StdError),

    // #[error("{0}")]
}

#[derive(Error, Debug, PartialEq)]
#[error(
    "Invalid inputed tokens proportion.\
     Expected: {expected_amount1}/{expected_amount0}.\
     Got: {got_amount1}/{got_amount0}"
)]
pub struct InvalidProportionError {
    pub expected_amount0: String, pub expected_amount1: String,
    pub got_amount0: String, pub got_amount1: String
}


// `#[invariant("...")]` macro?
pub enum Invariant {
    LoadedConstant(String)
}
