// SPDX-License-Identifier: MIT

use super::allowed_target;

#[test]
fn reference_rules_follow_the_referring_segment_type() {
    assert!(allowed_target(0, 53));
    assert!(allowed_target(20, 16));
    assert!(allowed_target(40, 4));
    assert!(allowed_target(62, 48));
    assert!(!allowed_target(0, 16));
    // Page information, end-of-page, and other segment types never refer.
    for source in [48, 49, 50, 51, 52, 53] {
        assert!(!allowed_target(source, 0));
    }
}
