use cosmwasm_std::Uint128;
use thiserror::Error;
use crate::constants::TWAP_SECONDS;

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
    ProtocolOperation(#[from] ProtocolOperationError),

    #[error("Cw20 error: {0}")]
    Cw20(#[from] cw20_base::ContractError)
}

#[derive(Error, Debug, PartialEq)]
pub enum InstantiationError {
    #[error("Vault creation costs {cost} of token {denom}, got: {got}")]
    VaultCreationCostNotPaid { cost: String, denom: String, got: String },

    #[error("Invalid concentrated liquidity pool_id {0}")]
    InvalidPoolId(u64),

    #[error("Invalid delegate vault rebalancer address: {0}")]
    InvalidDelegateAddress(String),

    #[error("Invalid vault admin address: {0}")]
    InvalidAdminAddress(String),

    #[error("Invalid vault admin fee: max: {max}; got: {got}")]
    InvalidAdminFee { max: Uint128, got: Uint128 },

    #[error("The vault admin cant have any fee if the vault doesnt have any admin")]
    AdminFeeWithoutAdmin { },

    #[error("Contradiction: {reason}; Hint: {hint}")]
    ContradictoryConfig { reason: String, hint: String },

    #[error("Price factors are Uint128 Decimals greater than 1, got: {0}")]
    InvalidPriceFactor(Uint128),

    #[error("Weights are Uint128 Decimals in the range [0, 1], got: {0}")]
    InvalidWeight(Uint128),
}

#[derive(Error, Debug, PartialEq)]
pub enum DepositError {
    #[error("The vault can only handle tokens {denom0} and {denom1}, but got: {unexpected}")]
    ImproperTokensSent { denom0: String, denom1: String, unexpected: String },

    #[error("Cant mint vault shares to itself ({0})")]
    ShareholderCantBeContract(String),

    #[error("Shareholder address for the deposit is not a valid address: {0}")]
    InvalidShareholderAddress(String),

    #[error("Used amounts below min wanted amounts: used: {used}, wanted: {wanted}")]
    DepositedAmountsBelowMin { used: String, wanted: String },

    #[error("Deposit must be above {min_liquidity}, got: {got}")]
    DepositedAmountBelowMinLiquidity { min_liquidity: Uint128, got: String }
}

#[derive(Error, Debug, PartialEq)]
pub enum RebalanceError {
    #[error("Only admin ({admin}) can rebalance, tried to rebalance from {got}")]
    UnauthorhizedNonAdminAccount { admin: String, got: String },

    #[error("Only the delegate address {delegate} can rebalance, tried to do so from {got}")]
    UnauthorizedDelegateAccount { delegate: String, got: String },

    #[error("Rebalancing the same vault twice per block is not supported, wait for the next block")]
    CantRebalanceTwicePerBlock(),

    #[error("Cant rebalance, price hasnt moved enough (price: {price}; movement_factor: {factor})")]
    PriceHasntMovedEnough { price: Uint128, factor: Uint128 },

    #[error("Cant rebalance, the price {price} moved outside [{twap}*0.99, {twap}*1.01]")]
    PriceMovedTooMuchInLastMinute { price: Uint128, twap: Uint128 },

    #[error("Cant rebalance pools that were created less than {TWAP_SECONDS} seconds ago")]
    PoolWasJustCreated(),

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
    InvalidWithdrawalAmount { owned: Uint128, withdrawn: Uint128 },

    #[error("Withdrawn amounts below min wanted amounts: got: {got}, wanted: {wanted}")]
    WithdrawnAmontsBelowMin { got: String, wanted: String }
}

#[derive(Error, Debug, PartialEq)]
pub enum ProtocolOperationError {
    #[error("Cant do protocol operation from non protocol account {0}")]
    UnauthorizedProtocolAccount(String),

    #[error("Invalid protocol fee: max: {max}; got: {got}")]
    InvalidProtocolFee { max: Uint128, got: Uint128 },
}

#[derive(Error, Debug, PartialEq)]
pub enum AdminOperationError {
    #[error("Cant do admin operations from non admin account {0}")]
    UnauthorizedAdminAccount(String),

    #[error("Cant do admin operations if vault has no admin")]
    NonExistantAdmin(),

    #[error("Invalid new proposed vault admin address: {0}")]
    InvalidNewProposedAdminAddress(String),

    #[error("There is no vault admin migration happening at this time")]
    NewProposedAdminIsNone(),

    #[error("Only the new proposed admin {expected} can take control of the vault, but {got} tried to")]
    UnauthorizedNewProposedAdmin { expected: String, got: String },

    // FIXME: `InstantiationError` has variants that will never happen here.
    //        Properly structure instantiation errors to prevent this.
    #[error("Tried to improperly reinstantiate state: {0}")]
    ReInstantiation(#[from] InstantiationError),

    #[error("Cant burn admin if the vault admin fee is not 0")]
    BurningAdminWithNonZeroAdminFee(),

    #[error("Tried to burn admin, but there are still uncollected admin fees")]
    BurningAdminWithUncollectedAdminFees(),

    #[error("Cant burn admin if the vault rebalancer is not Anyone")]
    BurningAdminWithImproperRebalancer(),

    #[error("Cant burn admin if the vault has a proposed new admin")]
    BurningAdminWithProposedNewAdmin()
}

