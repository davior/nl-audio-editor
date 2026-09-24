//! The WebAssembly build must reproduce the native build bit for bit.

use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn wasm_render_matches_the_native_hashes() {
    let (resolved, render) = nlae_core::parity::run();
    assert_eq!(
        resolved,
        nlae_core::parity::EXPECTED_RESOLVED,
        "resolution differs from native"
    );
    assert_eq!(
        render,
        nlae_core::parity::EXPECTED_RENDER,
        "render differs from native"
    );
}
