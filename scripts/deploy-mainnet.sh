#!/bin/sh

# ./scripts/mainnet.sh

WALLET=`osmosisd keys show -a magma-deployer`
echo "Compiling"

sudo docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.16.0

echo "Deploying from $WALLET"
TARGET=./artifacts/magma_core.wasm
osmosisd tx wasm store $TARGET \
  --from magma-deployer \
  --gas-prices 0.1uosmo \
  --gas auto \
  --gas-adjustment 1.3 \
  -y


