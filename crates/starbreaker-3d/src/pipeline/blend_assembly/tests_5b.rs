//! Phase 5B tests for scene creation with light datablocks.

use super::*;

#[test]
fn test_create_scene_blend_with_single_light() {
    let light = ExtractedLight {
        name: "TestLight".to_string(),
        parent_empty_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        color: [1.0, 1.0, 1.0],
        lamp_type: 0,
        energy_watts: 100.0,
        radius: 10.0,
        radius_source: 10.0,
        spot_size: 0.0,
        spot_blend: 0.0,
        intensity_candela: 5.0,
        temperature_k: 3000.0,
        use_temperature: false,
        gobo_path: None,
        active_state: "default".to_string(),
    };
    let result = create_scene_blend("TestWithLight", 1, "Data/Objects", &[light]);
    assert!(result.is_ok(), "Should create scene with light");
}

#[test]
fn test_create_scene_blend_with_multiple_lights() {
    let lights = vec![
        ExtractedLight {
            name: "Ambient".to_string(),
            parent_empty_name: None,
            parent_empty_loc: [0.0, 0.0, 0.0],
            parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
            parent_empty_scale: [1.0, 1.0, 1.0],
            position_blend: [0.0, 0.0, 0.0],
            rotation_blend: [1.0, 0.0, 0.0, 0.0],
            color: [0.8, 0.8, 0.8],
            lamp_type: 0,
            energy_watts: 50.0,
            radius: 20.0,
            radius_source: 20.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 5.0,
            temperature_k: 3000.0,
            use_temperature: false,
            gobo_path: None,
            active_state: "default".to_string(),
        },
        ExtractedLight {
            name: "Sun".to_string(),
            parent_empty_name: None,
            parent_empty_loc: [0.0, 0.0, 0.0],
            parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
            parent_empty_scale: [1.0, 1.0, 1.0],
            position_blend: [10.0, 10.0, 10.0],
            rotation_blend: [0.707, 0.0, 0.707, 0.0],
            color: [1.0, 1.0, 1.0],
            lamp_type: 1,
            energy_watts: 100.0,
            radius: 100.0,
            radius_source: 100.0,
            spot_size: 0.0,
            spot_blend: 0.0,
            intensity_candela: 100000.0,
            temperature_k: 5500.0,
            use_temperature: true,
            gobo_path: None,
            active_state: "default".to_string(),
        },
    ];
    let result = create_scene_blend("MultiLight", 1, "Data/Objects", &lights);
    assert!(result.is_ok(), "Should create scene with multiple lights");
}

#[test]
fn test_create_scene_blend_lights_in_file() {
    let light = ExtractedLight {
        name: "FileTest".to_string(),
        parent_empty_name: None,
        parent_empty_loc: [0.0, 0.0, 0.0],
        parent_empty_quat: [1.0, 0.0, 0.0, 0.0],
        parent_empty_scale: [1.0, 1.0, 1.0],
        position_blend: [0.0, 0.0, 0.0],
        rotation_blend: [1.0, 0.0, 0.0, 0.0],
        color: [1.0, 1.0, 1.0],
        lamp_type: 0,
        energy_watts: 100.0,
        radius: 10.0,
        radius_source: 10.0,
        spot_size: 0.0,
        spot_blend: 0.0,
        intensity_candela: 10.0,
        temperature_k: 3000.0,
        use_temperature: false,
        gobo_path: None,
        active_state: "default".to_string(),
    };
    let result = create_scene_blend("FileTest", 1, "Data/Objects", &[light]);
    assert!(result.is_ok());
    let blend_bytes = result.unwrap();
    let lamp_marker = b"LA\0\0";
    let lamp_count = blend_bytes.windows(4).filter(|w| *w == lamp_marker).count();
    assert!(lamp_count >= 1, "Should have at least 1 lamp block");
}

#[test]
fn test_create_scene_blend_no_lights() {
    let result = create_scene_blend("NoLights", 1, "Data/Objects", &[]);
    assert!(result.is_ok(), "Should work without lights");
}
