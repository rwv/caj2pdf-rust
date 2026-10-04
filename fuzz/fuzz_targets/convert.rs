// SPDX-License-Identifier: MIT
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| caj2pdf_fuzz::convert(data));
