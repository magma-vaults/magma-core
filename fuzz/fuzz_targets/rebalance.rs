#![no_main]

use libfuzzer_sys::fuzz_target;
extern crate magma_core;

use magma_core::mock::mock::{vault_params, PoolMockup};
use osmosis_test_tube::OsmosisTestApp;

/*
fuzz_target!(|data: &[u8]| {
    if data.len() == 7 { 
        panic!("TEST")
    }
});
*/

fuzz_target!(|_data: &[u8]| {
    // TODO: Generalize args.
    assert!(true);
    let _ = OsmosisTestApp::new();
    // let pool_mockup = PoolMockup::new(100_000, 200_000);
});
