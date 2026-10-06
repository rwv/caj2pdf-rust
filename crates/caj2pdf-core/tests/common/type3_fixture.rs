// SPDX-License-Identifier: MIT

// Original synthetic five-segment type-3 bytes, MQ-coded for the standard
// T.88 states by the test-only encoder. The including module provides
// `mq_encoder`.

use super::mq_encoder;

pub(super) fn segment(number: u8, kind: u8, refs: &[u8], data: &[u8]) -> Vec<u8> {
    let retain = 1 | (((1 << refs.len()) - 1) << 1);
    let mut bytes = vec![0, 0, 0, number, kind, ((refs.len() as u8) << 5) | retain];
    bytes.extend_from_slice(refs);
    bytes.push(1);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

pub(super) fn dib(width: u32, height: u32) -> [u8; 48] {
    let mut bytes = [0; 48];
    bytes[..4].copy_from_slice(&40_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&1_u16.to_le_bytes());
    bytes[32..36].copy_from_slice(&2_u32.to_le_bytes());
    bytes[40..43].fill(0xff);
    bytes
}

pub(super) fn dictionary_data(flags: u16) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&flags.to_be_bytes());
    data.extend_from_slice(&[2, 0xff]); // template-2 adaptive position (2,-1)
    data.extend_from_slice(&0_u32.to_be_bytes()); // exported symbols
    data.extend_from_slice(&0_u32.to_be_bytes()); // new symbols
    let mut body = mq_encoder();
    body.integer(0, Some(0)); // one zero IAEX run
    data.extend(body.finish());
    data
}

pub(super) fn page_data(width: u32, height: u32) -> [u8; 19] {
    let mut bytes = [0_u8; 19];
    bytes[..4].copy_from_slice(&width.to_be_bytes());
    bytes[4..8].copy_from_slice(&height.to_be_bytes());
    bytes[8..12].copy_from_slice(&3000_u32.to_be_bytes());
    bytes[12..16].copy_from_slice(&4000_u32.to_be_bytes());
    bytes[16] = 1; // default zero, OR, no striping
    bytes
}

pub(super) fn text_data(width: u32, height: u32, flags: u16) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.push(0); // external OR
    data.extend_from_slice(&flags.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes()); // zero text instances
    let mut body = mq_encoder();
    body.integer(0, Some(-4)); // STRIPT
    data.extend(body.finish());
    data
}

pub(super) fn generic_data(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.extend_from_slice(&[0, 4, 2, 0xff]); // OR, template 2, AT (2,-1)
    // Only the top-left pixel is black.
    let mut rows = vec![vec![false; width as usize]; height as usize];
    rows[0][0] = true;
    let mut body = mq_encoder();
    body.template2(0, &rows);
    data.extend(body.finish());
    data
}

pub(super) fn payload(width: u32, height: u32, text_flags: u16) -> Vec<u8> {
    let mut payload = dib(width, height).to_vec();
    payload.extend(
        [
            segment(0, 48, &[], &page_data(width, height)),
            segment(1, 0, &[], &dictionary_data(0x0800)),
            segment(2, 0, &[1], &dictionary_data(0x1802)),
            segment(3, 6, &[2], &text_data(width, height, text_flags)),
            segment(4, 38, &[], &generic_data(width, height)),
        ]
        .concat(),
    );
    payload
}
