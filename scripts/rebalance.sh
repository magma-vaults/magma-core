#!/bin/bash
CONTRACT_ADDR=$1
WALLET=`osmosisd keys show -a wallet`
MSG='{"rebalance": {}}'

osmosisd tx wasm execute $CONTRACT_ADDR "$MSG" \
    --from wallet \
    --gas-prices 0.025uosmo \
    --gas auto \
    --gas-adjustment 1.3 \
    --trace \
    -y
    
