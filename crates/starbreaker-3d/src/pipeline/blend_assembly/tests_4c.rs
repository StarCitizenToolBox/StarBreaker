//! Phase 4C tests for validation helpers across lights, empties, and decals.

use super::*;

#[test]
fn test_validate_lights_extraction_empty() {
    let lights = vec![];
    let result = validate_lights_extraction(&lights).unwrap();
    
    assert_eq!(result.light_count, 0);
    assert!(!result.is_valid);
    assert!(result.errors.len() > 0);
}

#[test]
fn test_validate_lights_extraction_single_ambient() {
    let lights = vec![
        ExtractedLight {
            name: "Ambient_001".to_string(),
            parent_empty_name: None,
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
        },
    ];
    
    let result = validate_lights_extraction(&lights).unwrap();
    assert_eq!(result.light_count, 1);
    assert!(result.lights_by_type.contains_key("Ambient"));
    assert_eq!(result.lights_by_type["Ambient"], 1);
}

#[test]
fn test_validate_lights_extraction_categorization() {
    let lights = vec![
        ExtractedLight {
            name: "Ambient".to_string(),
            parent_empty_name: None,
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
        },
        ExtractedLight {
            name: "Projector".to_string(),
            parent_empty_name: None,
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
            gobo_path: None,
            active_state: "defaultState".to_string(),
            states_json: None,
        },
    ];
    
    let result = validate_lights_extraction(&lights).unwrap();
    assert_eq!(result.light_count, 2);
    assert_eq!(result.lights_by_type.get("Ambient").unwrap_or(&0), &1);
    assert_eq!(result.lights_by_type.get("Projector").unwrap_or(&0), &1);
}

#[test]
fn test_validate_empties_extraction_empty() {
    let empties = vec![];
    let result = validate_empties_extraction(&empties).unwrap();
    
    assert_eq!(result.empty_count, 0);
    assert!(result.warnings.len() > 0);
}

#[test]
fn test_validate_empties_extraction_valid() {
    let empties = vec![
        ExtractedEmpty {
            name: "Helper_001".to_string(),
            nmc_index: 0,
            parent_nmc_index: None,
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            geometry_type: 3,
            is_helper: true,
        },
    ];
    
    let result = validate_empties_extraction(&empties).unwrap();
    assert_eq!(result.empty_count, 1);
    assert!(result.is_valid);
}

#[test]
fn test_validate_decals_extraction_valid() {
    let decals = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal_001".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 0,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decals_extraction(&decals).unwrap();
    assert_eq!(result.meshes_with_decals, 1);
    assert!(result.is_valid);
}

#[test]
fn test_validate_complete_phase_3_4_export_full() {
    let lights = vec![
        ExtractedLight {
            name: "Light1".to_string(),
            parent_empty_name: None,
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
        },
    ];
    
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
    ];
    
    let decals = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 0,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_complete_phase_3_4_export(&lights, &empties, &decals);
    assert_eq!(result.light_count, 1);
    assert_eq!(result.empty_count, 1);
    assert_eq!(result.meshes_with_decals, 1);
    assert!(result.is_valid);
}

#[test]
fn test_extracted_light_has_use_temperature_field() {
    // Test 1: Verify ExtractedLight struct has use_temperature field
    let light = ExtractedLight {
        name: "Test".to_string(),
        parent_empty_name: None,
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
        use_temperature: true,  // Verify field exists and can be set
        gobo_path: None,
        active_state: "defaultState".to_string(),
            states_json: None,
    };
    
    assert_eq!(light.temperature_k, 6500.0);
    assert_eq!(light.use_temperature, true);
}

#[test]
fn test_extracted_light_temperature_false() {
    // Test 2: use_temperature can be set to false
    let light = ExtractedLight {
        name: "Test".to_string(),
        parent_empty_name: None,
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
        temperature_k: 3000.0,
        use_temperature: false,  // Explicitly false
        gobo_path: None,
        active_state: "defaultState".to_string(),
            states_json: None,
    };
    
    assert_eq!(light.temperature_k, 3000.0);
    assert_eq!(light.use_temperature, false);
}

#[test]
fn test_temperature_values_range() {
    // Test 3: Temperature values within typical Kelvin range
    let test_temps = vec![
        (2700.0, true, "Warm white"),
        (3000.0, false, "Warm incandescent"),
        (5000.0, true, "Mid-range"),
        (6500.0, false, "Daylight"),
        (9000.0, true, "Cool white"),
        (12000.0, false, "Very cool"),
    ];
    
    for (temp, use_temp, desc) in test_temps {
        let light = ExtractedLight {
            name: format!("Light_{}", temp as i32),
            parent_empty_name: None,
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
            temperature_k: temp,
            use_temperature: use_temp,
            gobo_path: None,
            active_state: "defaultState".to_string(),
            states_json: None,
        };
        
        assert_eq!(light.temperature_k, temp, "Failed for {}: {}", temp, desc);
        assert_eq!(light.use_temperature, use_temp, "Failed for {}: {}", temp, desc);
    }
}
