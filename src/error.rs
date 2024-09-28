use thiserror::Error;
 
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("Entry point {0} is not payable")]
    NonPayable(String),

    #[error("Instantiation error: {0}")]
    Instantiation(#[from] InstantiationError),

    #[error("Deposit error: {0}")]
    Deposit(#[from] DepositError),

    #[error("Rebalance error: {0}")]
    Rebalance(#[from] RebalanceError),

    #[error("Withdrawal error: {0}")]
    Withdrawal(#[from] WithdrawalError),

    #[error("Admin operation error: {0}")]
    AdminOperation(#[from] AdminOperationError),

    #[error("Protocol operation error: {0}")]
    ProtocolOperation(#[from] ProtocolOperationError)
}

#[derive(Error, Debug, PartialEq)]
pub enum InstantiationError {
    #[error("Invalid concentrated liquidity pool_id {0}")]
    InvalidPoolId(u64),

    #[error("Invalid delegate vault rebalancer address: {0}")]
    InvalidDelegateAddress(String),

    #[error("Invalid vault admin address: {0}")]
    InvalidAdminAddress(String),

    #[error("Invalid vault admin fee: max: {max}; got: {got}")]
    InvalidAdminFee { max: String, got: String },

    #[error("The vault admin cant have any fee if the vault doesnt have any admin")]
    AdminFeeWithoutAdmin { },

    #[error("Contradiction: {reason}")]
    ContradictoryConfig { reason: String },

    #[error("Price factors are String Decimals greater than 1, got: {0}")]
    InvalidPriceFactor(String),

    #[error("Weights are String Decimals in the range [0, 1], got: {0}")]
    InvalidWeight(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum DepositError {
    // FIXME I wanted to ask for the inputs twice (swiss cheese model),
    //       but it do looks quite ugly, and stuff like this error only
    //       make the code more confusing. Remember, security comes with
    //       consistent semantics.
    #[error("Improper balances: expected {expected} but got {got}")]
    ImproperSentAmounts { expected: String, got: String },

    #[error("Nothing to deposit, user sent 0 tokens")]
    ZeroTokensSent {},

    #[error("Cant mint vault shares to itself ({0})")]
    ShareholderCantBeContract(String),

    #[error("Shareholder address for the deposit is not a valid address: {0}")]
    InvalidShareholderAddress(String),

    #[error("Used amounts below min wanted amounts: used: {used}, wanted: {wanted}")]
    DepositedAmontsBelowMin { used: String, wanted: String }
}

#[derive(Error, Debug, PartialEq)]
pub enum RebalanceError {
    #[error("Only admin ({admin}) can rebalance, tried to rebalance from {got}")]
    UnauthorhizedNonAdminAccount { admin: String, got: String },

    #[error("Only the delegate address {delegate} can rebalance, tried to do so from {got}")]
    UnauthorizedDelegateAccount { delegate: String, got: String },

    #[error("Cant rebalance, price hasnt moved enough (price: {price}; movement_factor: {factor})")]
    PriceHasntMovedEnough { price: String, factor: String },

    #[error("Not enough time passed since last rebalance, can rebalance in {time_left}")]
    NotEnoughTimePassed { time_left: u64 },

    #[error("You cant rebalance a vault without funds")]
    NothingToRebalance {},

    #[error("Pool with id {0} is empty, and thus has no price")]
    PoolWithoutPrice(u64),
}

#[derive(Error, Debug, PartialEq)]
pub enum WithdrawalError {
    #[error("Cant withdraw 0 shares")]
    ZeroSharesWithdrawal {},

    #[error("Trying to withdraw to improper address {0}")]
    InvalidWithdrawalAddress(String),
    
    #[error("Cant withdraw to itself ({0})")]
    CantWithdrawToContract(String),

    #[error("Trying to withdraw more shares than owned (owned: {owned}, withdrawn: {withdrawn})")]
    InalidWithdrawalAmount { owned: String, withdrawn: String },

    #[error("Withdrawn amounts below min wanted amounts: got: {got}, wanted: {wanted}")]
    WithdrawnAmontsBelowMin { got: String, wanted: String }
}

#[derive(Error, Debug, PartialEq)]
pub enum ProtocolOperationError {
    #[error("Cant claim protocol fees from non protocol account")]
    UnauthorizedProtocolAccount { },
}

#[derive(Error, Debug, PartialEq)]
pub enum AdminOperationError {
    #[error("Cant claim admin fees from non admin account")]
    UnauthorizedAdminAccount { },

    #[error("Cant claim admin fees if vault has no admin")]
    AdminFeesClaimForNonExistantAdmin { },
}
