#![no_main]

extern crate libfuzzer_sys;

use libfuzzer_sys::fuzz_target;
extern crate magma_core;
use magma_core::contract;

fuzz_target!(|data: &[u8]| {
    // TODO
});
