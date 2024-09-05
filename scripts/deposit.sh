#!/bin/bash
CONTRACT_ADDR=$1
WALLET=`osmosisd keys show -a wallet`
AMOUNT=69420

MSG='{"deposit": {"amount0": "0", "amount1": "'$AMOUNT'", "amount0_min": "0", "amount1_min": "'$AMOUNT'", "to": "'$WALLET'"}}'

osmosisd tx wasm execute $CONTRACT_ADDR "$MSG" --amount $AMOUNT'uosmo' \
    --from wallet \
    --gas-prices 0.025uosmo \
    --gas auto \
    --gas-adjustment 1.3 \
    -y
    
