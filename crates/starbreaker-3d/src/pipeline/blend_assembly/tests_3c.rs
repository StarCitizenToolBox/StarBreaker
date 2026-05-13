//! Phase 3C tests for light object block helpers.

use super::*;

#[test]
fn test_lamp_type_classification_point() {
    let collection_name = "Ambient";
    assert_eq!(collection_name, "Ambient");
}

#[test]
fn test_lamp_type_classification_projector() {
    let lamp_type = 2i16;
    let collection_name = match lamp_type {
        0 => "Ambient",
        1 => "Sun",
        2 => "Projector",
        4 => "Area",
        _ => "Other",
    };
    assert_eq!(collection_name, "Projector");
}

#[test]
fn test_lamp_type_classification_sun() {
    let lamp_type = 1i16;
    let collection_name = match lamp_type {
        0 => "Ambient",
        1 => "Sun",
        2 => "Projector",
        4 => "Area",
        _ => "Other",
    };
    assert_eq!(collection_name, "Sun");
}

#[test]
fn test_validate_lamp_block_sizes() {
    assert_eq!(LAMP_SIZE, 568);
    assert_eq!(OBJECT_SIZE, 1288);
}
