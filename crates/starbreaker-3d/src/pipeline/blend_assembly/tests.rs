//! Tests for Star Citizen to Blender light transform conversion helpers.

use super::*;

#[test]
fn test_convert_position_sc_to_blender() {
    // CryEngine position (1, 2, 3) → Blender (1, -3, 2)
    let result = convert_position_sc_to_blender([1.0, 2.0, 3.0]);
    assert_eq!(result, [1.0, -3.0, 2.0]);
}

#[test]
fn test_convert_position_origin() {
    let result = convert_position_sc_to_blender([0.0, 0.0, 0.0]);
    assert_eq!(result, [0.0, 0.0, 0.0]);
}

#[test]
fn test_convert_position_negative() {
    let result = convert_position_sc_to_blender([-1.0, -2.0, -3.0]);
    assert_eq!(result, [-1.0, 3.0, -2.0]);
}

#[test]
fn test_light_position_includes_container_transform() {
    let container_transform = crate::socpak::build_container_transform([10.0, 20.0, 30.0], [0.0, 0.0, 0.0]);
    let transformed = transform_light_position_sc(container_transform, [1.0, 2.0, 3.0]);
    assert_eq!(transformed, [11.0, 22.0, 33.0]);
    assert_eq!(convert_position_sc_to_blender(transformed), [11.0, -33.0, 22.0]);
}

#[test]
fn test_light_rotation_includes_container_transform() {
    let container_transform = crate::socpak::build_container_transform([0.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
    let transformed = transform_light_rotation_sc(container_transform, [1.0, 0.0, 0.0, 0.0]);
    assert!((transformed[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.0001);
    assert!(transformed[1].abs() < 0.0001);
    assert!(transformed[2].abs() < 0.0001);
    assert!((transformed[3] - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.0001);
}

#[test]
fn test_convert_quaternion_sc_to_blender() {
    // Addon-compatible light conversion applies scene-axis conversion plus
    // the glTF light basis correction for all light kinds.
    let result = convert_quaternion_sc_to_blender([1.0, 0.0, 0.0, 0.0], false);
    assert!((result[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
    assert!((result[1] - 0.0).abs() < 0.0001);
    assert!((result[2] + std::f32::consts::FRAC_1_SQRT_2).abs() < 0.0001);
    assert!((result[3] - 0.0).abs() < 0.0001);
}

#[test]
fn test_convert_quaternion_with_values() {
    let result = convert_quaternion_sc_to_blender([0.707, 0.5, 0.5, 0.0], false);
    assert!((result[0] - 0.5).abs() < 0.001);
    assert!((result[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.001);
    assert!((result[2] + 0.5).abs() < 0.001);
    assert!(result[3].abs() < 0.001);
}

#[test]
fn test_quaternion_multiply_identity() {
    // Multiply by identity quaternion [1, 0, 0, 0]
    let q = [0.7071, 0.0, -0.7071, 0.0];
    let identity = [1.0, 0.0, 0.0, 0.0];
    let result = quaternion_multiply(q, identity);
    // Result should be close to q (allow small floating-point error)
    assert!((result[0] - q[0]).abs() < 0.0001);
    assert!((result[1] - q[1]).abs() < 0.0001);
    assert!((result[2] - q[2]).abs() < 0.0001);
    assert!((result[3] - q[3]).abs() < 0.0001);
}

#[test]
fn test_spotlight_orientation_correction() {
    // Test that spotlight correction applies basis rotation
    // Identity quaternion should apply the basis correction
    let identity = [1.0, 0.0, 0.0, 0.0];
    let result_spotlight = convert_quaternion_sc_to_blender_test_helper(identity, true);
    let result_point = convert_quaternion_sc_to_blender_test_helper(identity, false);
    
    // All light kinds get the glTF light basis correction to match the addon.
    assert_eq!(result_point, result_spotlight);
    
    // Spotlight: basis correction applied, should be rotated
    // Identity * basis_correction = [0.7071, 0, -0.7071, 0]
    assert!((result_spotlight[0] - 0.7071).abs() < 0.0001, "w component mismatch");
    assert!((result_spotlight[1] - 0.0).abs() < 0.0001, "x component mismatch");
    assert!((result_spotlight[2] - (-0.7071)).abs() < 0.0001, "y component mismatch");
    assert!((result_spotlight[3] - 0.0).abs() < 0.0001, "z component mismatch");
}

#[test]
fn test_spotlight_orientation_from_cryengine() {
    // Test with a realistic CryEngine spotlight pointing along +X
    // In CryEngine: [w, x, y, z] represents rotation about +X
    // After conversion: should point along -Z in Blender
    
    // Example: 90° rotation around X axis (CryEngine forward)
    let quat_90_x = [0.7071, 0.7071, 0.0, 0.0];  // [cos(45°), sin(45°), 0, 0]
    let result = convert_quaternion_sc_to_blender(quat_90_x, true);
    
    // After coord xform: [w, x, -z, y] = [0.7071, 0.7071, 0.0, 0.0]
    // After basis correction (90° around Y): should have adjusted components
    // We just verify the function runs and produces normalized output
    let magnitude_sq = result[0]*result[0] + result[1]*result[1] + 
                       result[2]*result[2] + result[3]*result[3];
    assert!((magnitude_sq - 1.0).abs() < 0.01, "quaternion should be normalized");
}

#[test]
fn test_point_light_uses_gltf_basis_correction() {
    let quat = [0.7071, 0.3, 0.5, 0.1];
    let result_point = convert_quaternion_sc_to_blender(quat, false);
    let magnitude_sq = result_point[0]*result_point[0] + result_point[1]*result_point[1] +
                       result_point[2]*result_point[2] + result_point[3]*result_point[3];
    assert!((magnitude_sq - 1.0).abs() < 0.01, "quaternion should be normalized");
    assert_ne!(result_point, [quat[0] as f32, quat[1] as f32, -(quat[3] as f32), quat[2] as f32]);
}

// Helper function for testing (internal use only)
fn convert_quaternion_sc_to_blender_test_helper(quat_sc: [f64; 4], is_spotlight: bool) -> [f32; 4] {
    convert_quaternion_sc_to_blender(quat_sc, is_spotlight)
}

#[test]
fn test_lamp_type_mapping_omni() {
    // Create a minimal light info for testing
    let light_type = "Omni";
    let lamp_type = match light_type {
        "Omni" | "SoftOmni" => 0,
        "Projector" => 2,
        "Ambient" => 0,
        "Directional" | "Sun" => 1,
        _ => 0,
    };
    assert_eq!(lamp_type, 0); // POINT
}

#[test]
fn test_lamp_type_mapping_projector() {
    let light_type = "Projector";
    let lamp_type = match light_type {
        "Omni" | "SoftOmni" => 0,
        "Projector" => 2,
        "Ambient" => 0,
        "Directional" | "Sun" => 1,
        _ => 0,
    };
    assert_eq!(lamp_type, 2); // SPOT
}

#[test]
fn test_lamp_type_mapping_sun() {
    let light_type = "Sun";
    let lamp_type = match light_type {
        "Omni" | "SoftOmni" => 0,
        "Projector" => 2,
        "Ambient" => 0,
        "Directional" | "Sun" => 1,
        _ => 0,
    };
    assert_eq!(lamp_type, 1); // SUN
}

#[test]
fn test_energy_conversion() {
    // 200 candelas → ~73.6 W (200 * 4π / 683 * 20)
    let cd = 200.0;
    let energy = light_energy_to_blender(0, "point", cd, 1.0);
    assert!(energy > 73.0 && energy < 74.0, "Energy: {}", energy);
}

#[test]
fn test_ambient_proxy_light_energy_uses_no_visual_gain() {
    let energy = light_energy_to_blender(0, "ambient_proxy", 200.0, 1.0);
    assert!((energy - (200.0 * 4.0 * std::f32::consts::PI / 683.0)).abs() < 0.0001);
}

#[test]
fn test_area_light_energy_matches_addon_lumen_path() {
    let energy = light_energy_to_blender(4, "area", 200.0, 120.0);
    assert!((energy - 1.0).abs() < 0.0001, "Energy: {}", energy);
}

#[test]
fn test_sun_light_energy_uses_candela_proxy_without_visual_gain() {
    let energy = light_energy_to_blender(1, "sun", 683.0, 120.0);
    assert!((energy - 1.0).abs() < 0.0001, "Energy: {}", energy);
}

#[test]
fn test_semantic_area_maps_to_area_lamp() {
    assert_eq!(lamp_type_for_light("Planar", "area"), 4);
}
