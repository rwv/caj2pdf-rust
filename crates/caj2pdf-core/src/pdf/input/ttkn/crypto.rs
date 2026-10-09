// SPDX-License-Identifier: MIT

//! The measured wrapping CBC mode and standard PDF AESV2 object keys.

use aes::cipher::{BlockCipherDecrypt, KeyInit};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use zeroize::Zeroizing;

use crate::pdf::PdfRef;
use crate::{Error, ErrorKind, Result};

// Established on original synthetic inputs, then independently checked on
// the public sample. This is a format initializer, not a source credential.
pub(super) const WRAPPING_IV: [u8; 16] = *b"200CFC8299B84aa9";

pub(super) fn rejected() -> Error {
    Error::from(ErrorKind::Encrypted).because("TTKN response or encrypted data is invalid")
}

pub(super) fn object_key(file_key: &[u8; 16], reference: PdfRef) -> Zeroizing<[u8; 16]> {
    let mut digest = Md5::new();
    digest.update(file_key);
    digest.update(&reference.number.to_le_bytes()[..3]);
    digest.update(reference.generation.to_le_bytes());
    digest.update(b"sAlT");
    Zeroizing::new(digest.finalize().into())
}

pub(super) fn block(cipher: &Aes128, ciphertext: &[u8; 16], previous: &[u8; 16]) -> [u8; 16] {
    let mut block = (*ciphertext).into();
    cipher.decrypt_block(&mut block);
    for (byte, before) in block.iter_mut().zip(previous) {
        *byte ^= before;
    }
    block.into()
}

pub(super) fn unwrap(key: &[u8; 32], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(16) {
        return Err(rejected());
    }
    let cipher = Aes256::new(key.into());
    let mut previous = *iv;
    for chunk in bytes.as_chunks_mut::<16>().0 {
        let current = *chunk;
        let mut block = current.into();
        cipher.decrypt_block(&mut block);
        for ((target, decoded), before) in chunk.iter_mut().zip(block).zip(previous) {
            *target = decoded ^ before;
        }
        previous = current;
    }
    Ok(())
}

pub(super) fn padding(last: &[u8; 16]) -> Result<usize> {
    let count = usize::from(last[15]);
    if !(1..=16).contains(&count) || last[16 - count..].iter().any(|&b| usize::from(b) != count) {
        return Err(rejected());
    }
    Ok(count)
}

pub(super) fn decrypt_string(key: &[u8; 16], bytes: &mut Vec<u8>) -> Result<()> {
    // PDF writers may leave an empty string empty; it carries no ciphertext.
    if bytes.is_empty() {
        return Ok(());
    }
    if bytes.len() < 32 || !bytes.len().is_multiple_of(16) {
        return Err(rejected());
    }
    let cipher = Aes128::new(key.into());
    let mut previous: [u8; 16] = bytes[..16].try_into().expect("checked IV");
    for chunk in bytes[16..].as_chunks_mut::<16>().0 {
        let current = *chunk;
        chunk.copy_from_slice(&block(&cipher, &current, &previous));
        previous = current;
    }
    let last: &[u8; 16] = bytes[bytes.len() - 16..]
        .try_into()
        .expect("checked last block");
    let end = bytes.len() - padding(last)?;
    bytes.copy_within(16..end, 0);
    bytes.truncate(end - 16);
    Ok(())
}
