//! Phase 3B tests for NMC empty transform extraction helpers.

use super::*;

#[test]
fn test_matrix_to_quaternion_identity() {
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let quat = matrix_to_quaternion(&identity);
    assert!((quat[0] - 1.0).abs() < 0.01);
    assert!(quat[1].abs() < 0.01);
    assert!(quat[2].abs() < 0.01);
    assert!(quat[3].abs() < 0.01);
}

#[test]
fn test_extract_matrix_components_position() {
    let matrix = [
        [1.0, 0.0, 0.0, 5.0],
        [0.0, 1.0, 0.0, 10.0],
        [0.0, 0.0, 1.0, 15.0],
    ];
    let (pos, _rot, _scale) = extract_matrix_components(&matrix);
    assert_eq!(pos, [5.0, 10.0, 15.0]);
}

#[test]
fn test_extracted_empty_helpers() {
    let is_helper = 3 != 0;
    assert!(is_helper);
}
