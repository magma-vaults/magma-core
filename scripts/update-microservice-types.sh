#!/bin/sh

echo "Updating types to front-end repo"
echo "Make sure to have done `cargo run schema`"

if [ -d "ts" ]; then
    rm -rd ts
fi

node ./../ts-codegen/packages/ts-codegen/dist/ts-codegen.js generate \
    --schema ./schema \
    --out ./ts \
    --name MagmaCore \
    --plugin none \
    --no-bundle \
    --no-optionalClient \
    --no-queryKeys \
    --no-mutations

echo "Types generated, updating in microservice"

CURRENT_DIR=`pwd`
TARGET_DIR=~/work/magma-snapshots/src/types

rm $TARGET_DIR/*
cp ts/* $TARGET_DIR
rm -rf ts

