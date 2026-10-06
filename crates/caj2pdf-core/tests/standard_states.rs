// SPDX-License-Identifier: MIT

use caj2pdf_core::{jbig2::mq, qm};
use sha2::{Digest, Sha256};

#[test]
fn standard_qm_states_match_the_pinned_interoperability_data() {
    let mut hash = Sha256::new();
    for state in qm::STANDARD_STATES {
        hash.update(state.qe.to_be_bytes());
        hash.update([state.next_lps, state.next_mps, u8::from(state.switch_mps)]);
    }
    assert_eq!(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "30b944771e6f7815fdb1a6b9cb1864dd8d1403a96e19f33a82dbd65d42141220"
    );
}

#[test]
fn standard_mq_states_match_the_pinned_interoperability_data() {
    let mut hash = Sha256::new();
    for state in mq::STANDARD_STATES {
        hash.update(state.qe.to_be_bytes());
        hash.update([state.next_lps, state.next_mps, u8::from(state.switch_mps)]);
    }
    assert_eq!(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "cfcb2cd66ff102e77e2f74c758c4e1881b57ce6d1113b59a24cae1e9a07858c2"
    );
}
