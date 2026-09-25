// SPDX-License-Identifier: MIT

//! Dependency-free WebAssembly boundary for bounded browser and Node.js I/O.
//!
//! [`engine`] drives the same `caj2pdf-core` conversion futures as native
//! Rust through a poll/resume state machine. On `wasm32`, a private raw ABI
//! exposes that engine to the JavaScript package in `js/`.

// Raw exports require Rust 2024's `unsafe(no_mangle)` linkage marker. The
// implementation uses no unsafe blocks or pointer dereferences in Rust.

pub mod engine;

#[cfg(target_arch = "wasm32")]
mod bridge;
