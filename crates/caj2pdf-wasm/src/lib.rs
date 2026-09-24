// SPDX-License-Identifier: MIT

//! Dependency-free WebAssembly boundary for bounded browser and Node.js I/O.
//!
//! The raw ABI is an I/O proof. It drives the same `caj2pdf-core::copy_range`
//! future as native Rust; format conversion is implemented in later issues.

// Raw exports require Rust 2024's `unsafe(no_mangle)` linkage marker. The
// implementation uses no unsafe blocks or pointer dereferences in Rust.

#[cfg(target_arch = "wasm32")]
mod bridge;
