#!/bin/sh

# ./scripts/testnet.sh

WALLET=`osmosisd keys show -a wallet`
echo "Compiling"
export RUSTFLAGS='-C link-args=-s'
# cargo build --target wasm32-unknown-unknown --release --lib
cargo wasm

echo "Deploying from $WALLET"
TARGET=./target/wasm32-unknown-unknown/release/magma_core.wasm
osmosisd tx wasm store $TARGET \
  --from wallet \
  --gas-prices 0.1uosmo \
  --gas auto \
  --gas-adjustment 1.3 \
  -y


