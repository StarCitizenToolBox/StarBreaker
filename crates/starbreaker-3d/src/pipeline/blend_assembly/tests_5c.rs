//! Phase 5C tests for empty object collection organization.

use super::*;

#[test]
fn organize_empties_collection_basic() {
    let empties = vec![ExtractedEmpty {
        name: "Empty_Root".to_string(),
        nmc_index: 0,
        parent_nmc_index: None,
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        geometry_type: 0,
        is_helper: false,
    }];
    
    let tree = organize_empties_into_collections(&empties).unwrap();
    assert_eq!(tree.root_ptr, 0x1000);
    assert!(tree.type_collections.contains_key("Armature"));
    assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 1);
}

#[test]
fn organize_empties_collection_hierarchy() {
    let empties = vec![
        ExtractedEmpty {
            name: "Root".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Child".to_string(),
            nmc_index: 1,
            parent_nmc_index: Some(0),
            position_blend: [1.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
    ];
    
    let tree = organize_empties_into_collections(&empties).unwrap();
    assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 2);
    
    // Verify parent-child relationships preserved
    verify_empty_hierarchy_preservation(&empties).unwrap();
}

#[test]
fn organize_empties_collection_transforms() {
    let empties = vec![ExtractedEmpty {
        name: "TransformedEmpty".to_string(),
        nmc_index: 0,
        parent_nmc_index: None,
        position_blend: [1.5, -2.5, 3.0],
        rotation_blend: [0.707, 0.0, 0.707, 0.0],  // 90° rotation
        scale: [2.0, 2.0, 2.0],
        geometry_type: 3,
        is_helper: true,
    }];
    
    let tree = organize_empties_into_collections(&empties).unwrap();
    let controls_empties = tree.empties_by_collection.get("Controls").unwrap();
    assert_eq!(controls_empties.len(), 1);
    
    let empty = &controls_empties[0].empty;
    assert_eq!(empty.position_blend, [1.5, -2.5, 3.0]);
    assert_eq!(empty.scale, [2.0, 2.0, 2.0]);
}

#[test]
fn organize_empties_collection_deep_hierarchy() {
    let empties = vec![
        ExtractedEmpty {
            name: "Level0".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Level1".to_string(),
            nmc_index: 1,
            parent_nmc_index: Some(0),
            position_blend: [1.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Level2".to_string(),
            nmc_index: 2,
            parent_nmc_index: Some(1),
            position_blend: [2.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Level3".to_string(),
            nmc_index: 3,
            parent_nmc_index: Some(2),
            position_blend: [3.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Level4".to_string(),
            nmc_index: 4,
            parent_nmc_index: Some(3),
            position_blend: [4.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Level5".to_string(),
            nmc_index: 5,
            parent_nmc_index: Some(4),
            position_blend: [5.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
    ];
    
    let tree = organize_empties_into_collections(&empties).unwrap();
    assert_eq!(tree.empties_by_collection.get("Armature").unwrap().len(), 6);
    
    // Verify deep hierarchy
    verify_empty_hierarchy_preservation(&empties).unwrap();
}

#[test]
fn organize_empties_collection_no_duplicates() {
    let empties = vec![
        ExtractedEmpty {
            name: "Empty1".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 3,
            is_helper: true,
        },
        ExtractedEmpty {
            name: "Empty2".to_string(),
            nmc_index: 1,
            parent_nmc_index: None,
            position_blend: [1.0, 1.0, 1.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 3,
            is_helper: true,
        },
        ExtractedEmpty {
            name: "Empty3".to_string(),
            nmc_index: 2,
            parent_nmc_index: None,
            position_blend: [2.0, 2.0, 2.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 3,
            is_helper: true,
        },
    ];
    
    let tree = organize_empties_into_collections(&empties).unwrap();
    
    // All empties in Controls collection
    let controls = tree.empties_by_collection.get("Controls").unwrap();
    assert_eq!(controls.len(), 3);
    
    // Verify no duplicates
    let names: std::collections::HashSet<_> = controls
        .iter()
        .map(|e| e.empty.name.clone())
        .collect();
    assert_eq!(names.len(), 3);
}

#[test]
fn empty_category_classification_helpers() {
    let cat = EmptyCategory::from_empty(true, 0);
    assert_eq!(cat, EmptyCategory::Helpers);
    assert_eq!(cat.collection_name(), "Helpers");
}

#[test]
fn empty_category_classification_controls() {
    let cat = EmptyCategory::from_empty(true, 3);
    assert_eq!(cat, EmptyCategory::Controls);
    assert_eq!(cat.collection_name(), "Controls");
}

#[test]
fn empty_category_classification_armature() {
    let cat = EmptyCategory::from_empty(false, 0);
    assert_eq!(cat, EmptyCategory::Armature);
    assert_eq!(cat.collection_name(), "Armature");
}

#[test]
fn validate_empty_collection_hierarchy_valid() {
    let mut collections = std::collections::HashMap::new();
    collections.insert("Helpers".to_string(), 0x2000);
    collections.insert("Controls".to_string(), 0x2200);
    collections.insert("Armature".to_string(), 0x2400);
    
    let mut empties_by_collection = std::collections::HashMap::new();
    empties_by_collection.insert(
        "Helpers".to_string(),
        vec![OrganizedEmpty {
            empty: ExtractedEmpty {
                name: "Helper1".to_string(),
                nmc_index: 0,
                parent_nmc_index: None,
                position_blend: [0.0, 0.0, 0.0],
                rotation_blend: [1.0, 0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                geometry_type: 0,
                is_helper: true,
            },
            collection_name: "Helpers".to_string(),
        }],
    );
    
    let tree = EmptyCollectionTree {
        root_ptr: 0x1000,
        type_collections: collections,
        empties_by_collection,
    };
    
    assert!(validate_empty_collection_hierarchy(&tree).is_ok());
}

#[test]
fn verify_empty_hierarchy_preservation_valid() {
    let empties = vec![
        ExtractedEmpty {
            name: "Root".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
        ExtractedEmpty {
            name: "Child".to_string(),
            nmc_index: 1,
            parent_nmc_index: Some(0),
            position_blend: [1.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            is_helper: false,
        },
    ];
    
    assert!(verify_empty_hierarchy_preservation(&empties).is_ok());
}

#[test]
fn verify_empty_hierarchy_preservation_invalid_parent() {
    let empties = vec![ExtractedEmpty {
        name: "Orphan".to_string(),
        nmc_index: 0,
        parent_nmc_index: Some(999),  // Non-existent parent
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        geometry_type: 0,
        is_helper: false,
    }];
    
    assert!(verify_empty_hierarchy_preservation(&empties).is_err());
}
