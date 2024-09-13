
use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use thiserror::Error;
 
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("cw20 error: {0}")]
    Cw20(#[from] cw20_base::ContractError),

    #[error("Payment error: {0}")]
    Payment(#[from] PaymentError),

    // Instantiation errors.
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
    InvalidWeight(String),

    // Deposit errors.
    #[error("Improper balances: expected {expected} but got {got}")]
    ImproperSentAmounts { expected: String, got: String },

    #[error("Nothing to deposit, user sent 0 tokens")]
    ZeroTokensSent {},

    #[error("Cant mint vault shares to itself ({0})")]
    ShareholderCantBeContract(String),

    #[error("Used amounts below min wanted amounts: used: {used}, wanted: {wanted}")]
    DepositedAmontsBelowMin { used: String, wanted: String },

    #[error("Invalid shareholder address: {0}")]
    InvalidShareholderAddress(String),

    // Rebalance errors.
    #[error("You cant rebalance a vault without funds")]
    NothingToRebalance {},

    #[error("Pool with id {0} is empty, and thus has no price")]
    PoolWithoutPrice(u64),

    // Withdraw erors.
    #[error("Cant withdraw 0 shares")]
    ZeroSharesWithdrawal {},

    #[error("Trying to withdraw to improper address {0}")]
    InvalidWithdrawalAddress(String),
    
    #[error("Cant withdraw to itself ({0})")]
    CantWithdrawToContract(String),

    #[error("Trying to withdraw more shares than owned (owned: {owned}, withdrawn: {withdrawn})")]
    InalidWithdrawalAmount { owned: String, withdrawn: String },

    #[error("Withdrawn amounts below min wanted amounts: got: {got}, wanted: {wanted}")]
    WithdrawnAmontsBelowMin { got: String, wanted: String },
}

