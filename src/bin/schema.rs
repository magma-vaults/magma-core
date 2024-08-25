use cosmwasm_schema::write_api;
use magma_core::msg::{ExecuteMsg, InstantiateMsg};

fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg
    }
}

