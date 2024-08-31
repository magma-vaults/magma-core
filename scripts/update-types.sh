#!/bin/sh

echo "Updating types to front-end repo"
echo "Make sure to have done `cargo run schema`"

if [ -d "ts" ]; then
    rm -rd ts
fi

ts-codegen generate \
    --schema ./schema \
    --out ./ts \
    --name MagmaCore \
    --plugin react-query \
    --no-bundle \
    --no-optionalClient \
    --version v4 \
    --no-queryKeys \
    --no-mutations

echo "Types generated, cloning front-end"
git clone https://github.com/Artegus/magma-front

echo "Updating front-end types"
CURRENT_DIR=`pwd`

cd magma-front/src/lib

if [ -d "./generated-contract-types" ]; then
    rm ./generated-contract-types/*
else
    mkdir generated-contract-types
fi

cd generated-contract-types
TARGET_DIR=`pwd`

cd $CURRENT_DIR
cp ts/* $TARGET_DIR

echo "Commiting changes"
rm -rd ts
cd magma-front

DATE=`date +%s` # Current unix timestamp
git add .
git commit -m "generated contract types update at $DATE"
git push origin main

cd ..
rm -rf magma-front
