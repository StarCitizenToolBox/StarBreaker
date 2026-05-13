//! Phase 3E tests for light collection organization.

use super::*;

#[test]
fn test_light_category_ambient() {
    let cat = LightCategory::from_light(0, 5.0);
    assert_eq!(cat, LightCategory::Ambient);
    assert_eq!(cat.collection_name(), "Ambient");
}

#[test]
fn test_light_category_omni() {
    let cat = LightCategory::from_light(0, 50.0);
    assert_eq!(cat, LightCategory::Omni);
    assert_eq!(cat.collection_name(), "Omni");
}

#[test]
fn test_light_category_soft_omni() {
    let cat = LightCategory::from_light(0, 150.0);
    assert_eq!(cat, LightCategory::SoftOmni);
    assert_eq!(cat.collection_name(), "SoftOmni");
}

#[test]
fn test_light_category_projector() {
    let cat = LightCategory::from_light(2, 200.0);
    assert_eq!(cat, LightCategory::Projector);
    assert_eq!(cat.collection_name(), "Projector");
}

#[test]
fn test_light_category_sun() {
    let cat = LightCategory::from_light(1, 100.0);
    assert_eq!(cat, LightCategory::Sun);
    assert_eq!(cat.collection_name(), "Sun");
}

#[test]
fn test_organize_lights_empty() {
    let lights = vec![];
    let tree = organize_lights_into_collections(&lights).unwrap();
    assert_eq!(tree.root_ptr, 0x1000);
    assert_eq!(tree.type_collections.len(), 0);
}

#[test]
fn test_organize_lights_creates_type_collections() {
    let lights = vec![
        ExtractedLight {
            name: "Ambient1".to_string(),
            parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
            parent_empty_loc: [0.0, 0.0, 0.0],
            parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
            parent_empty_scale: [1.0, 1.0, 1.0],
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 0,
            energy_watts: 1.0,
            radius: 5.0,
            cutoff_distance: 5.0,
            radius_source: 5.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 6500.0,
            use_temperature: false,
            gobo_path: None,
            active_state: "defaultState".to_string(),
            states_json: None,
            semantic_light_kind: "point".to_string(),
        },
        ExtractedLight {
            name: "Projector1".to_string(),
            parent_empty_name: None,
        parent_empty_parent_entity_name: None,
        parent_empty_parent_node_name: None,
            parent_empty_loc: [0.0, 0.0, 0.0],
            parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
            parent_empty_scale: [1.0, 1.0, 1.0],
            position_blend: [1.0, 1.0, 1.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 0.8, 0.6],
            lamp_type: 2,
            energy_watts: 50.0,
            radius: 10.0,
            cutoff_distance: 10.0,
            radius_source: 10.0,
            spot_size: 0.5,
            spot_blend: 0.2,
            intensity_candela: 200.0,
            temperature_k: 6500.0,
            use_temperature: false,
            gobo_path: Some("path/to/gobo.dds".to_string()),
            active_state: "defaultState".to_string(),
            states_json: None,
            semantic_light_kind: "point".to_string(),
        },
    ];

    let tree = organize_lights_into_collections(&lights).unwrap();
    assert_eq!(tree.root_ptr, 0x1000);
    assert!(tree.type_collections.contains_key("Ambient"));
    assert!(tree.type_collections.contains_key("Projector"));
}

#[test]
fn test_validate_light_collection_hierarchy_valid() {
    let tree = LightCollectionTree {
        root_ptr: 0x1000,
        type_collections: vec![
            ("Ambient".to_string(), 0x2000),
            ("Projector".to_string(), 0x2200),
        ]
        .into_iter()
        .collect(),
    };

    assert!(validate_light_collection_hierarchy(&tree).is_ok());
}

#[test]
fn test_validate_light_collection_hierarchy_zero_root() {
    let tree = LightCollectionTree {
        root_ptr: 0,
        type_collections: std::collections::HashMap::new(),
    };

    assert!(validate_light_collection_hierarchy(&tree).is_err());
}

#[test]
fn test_validate_light_collection_hierarchy_duplicate_ptr() {
    let mut collections = std::collections::HashMap::new();
    collections.insert("Ambient".to_string(), 0x2000);
    collections.insert("Projector".to_string(), 0x2000);

    let tree = LightCollectionTree {
        root_ptr: 0x1000,
        type_collections: collections,
    };

    assert!(validate_light_collection_hierarchy(&tree).is_err());
}
