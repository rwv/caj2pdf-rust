// SPDX-License-Identifier: MIT

//! Dependency-free WebAssembly boundary for the browser and Node.js package.
//!
//! [`engine`] runs the same synchronous `caj2pdf-core` conversions as native
//! Rust over a [`engine::Host`] that performs ranged reads, ordered writes,
//! progress and cancellation checks. On `wasm32`, a private raw ABI exposes
//! it to the JavaScript package in `js/`, which runs it in a Worker.

// Raw exports require Rust 2024's `unsafe(no_mangle)` linkage marker, and the
// imported host functions are called in `unsafe` blocks that pass only
// pointers to live Rust-owned memory.

pub mod engine;

#[cfg(target_arch = "wasm32")]
mod bridge;
