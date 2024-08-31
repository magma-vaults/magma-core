#!/bin/sh

./scripts/testnet.sh

CODE_ID=$1
POOL_ID=537 # USDC/OSMO testnet pool
WALLET=`osmosisd keys show -a wallet`

INIT='{
    "vault_info": {
        "pool_id": '$POOL_ID',
        "vault_name": "My USDC/OSMO pool",
        "vault_symbol": "USDCOSMO",
        "admin": "'$WALLET'",
        "rebalancer": { "admin": {} }
    },
    "vault_parameters": {
        "base_factor": "2",
        "limit_factor": "1.5",
        "full_range_weight": "0.55"
    }
}'

echo "Instantiating from $WALLET"
osmosisd tx wasm instantiate $CODE_ID "$INIT" \
    --from wallet \
    --label "testnet vault" \
    --gas-prices 0.025uosmo \
    --gas auto \
    --gas-adjustment 1.3 \
    --no-admin \
    -y
