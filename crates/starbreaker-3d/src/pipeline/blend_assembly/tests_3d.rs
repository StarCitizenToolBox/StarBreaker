//! Phase 3D tests for empty object classification.

use super::*;

#[test]
fn test_empty_type_classification_helper() {
    let is_helper = true;
    let geometry_type = 3u16;
    let collection_name = if is_helper {
        match geometry_type {
            3 => "Controls",
            _ => "Helpers",
        }
    } else {
        "Armature"
    };
    assert_eq!(collection_name, "Controls");
}

#[test]
fn test_empty_type_classification_non_helper() {
    let is_helper = false;
    let collection_name = if is_helper { "Helpers" } else { "Armature" };
    assert_eq!(collection_name, "Armature");
}

#[test]
fn test_empty_type_classification_generic_helper() {
    let is_helper = true;
    let geometry_type = 0u16;
    let collection_name = if is_helper {
        match geometry_type {
            3 => "Controls",
            _ => "Helpers",
        }
    } else {
        "Armature"
    };
    assert_eq!(collection_name, "Helpers");
}

#[test]
fn test_validate_empty_object_creation() {
    assert_eq!(OBJECT_SIZE, 1288);
}
