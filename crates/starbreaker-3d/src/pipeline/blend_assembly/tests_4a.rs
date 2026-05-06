//! Phase 4A tests for decal material identification.

use super::*;

#[test]
fn test_validate_decal_material_identification_valid() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal_Glass".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 0,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_ok());
}

#[test]
fn test_validate_decal_material_identification_invalid_empty() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_err());
}

#[test]
fn test_validate_decal_material_identification_pom() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "POM_Rock".to_string(),
                    is_decal: false,
                    is_pom: true,
                    material_index: 1,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_ok());
}

#[test]
fn test_validate_decal_material_identification_both() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Decal_POM".to_string(),
                    is_decal: true,
                    is_pom: true,
                    material_index: 2,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_ok());
}

#[test]
fn test_validate_decal_material_identification_multiple_meshes() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_A".to_string(),
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
        MeshWithDecals {
            mesh_path: "Mesh_B".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "POM_Rock".to_string(),
                    is_decal: false,
                    is_pom: true,
                    material_index: 1,
                },
                DecalMaterial {
                    material_name: "Decal_Glass".to_string(),
                    is_decal: true,
                    is_pom: false,
                    material_index: 2,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_ok());
}

#[test]
fn test_validate_decal_material_identification_invalid_flags() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![
                DecalMaterial {
                    material_name: "Bad_Material".to_string(),
                    is_decal: false,
                    is_pom: false,  // Neither flag set - invalid!
                    material_index: 0,
                },
            ],
            decal_face_indices: vec![],
        },
    ];
    
    let result = validate_decal_material_identification(&meshes);
    assert!(result.is_err());
}

#[test]
fn test_identify_decal_material_flags_decal() {
    let (is_decal, is_pom) = identify_decal_material_flags("Shader { %DECAL %VERTEX_COLORS }");
    assert!(is_decal);
    assert!(!is_pom);
}

#[test]
fn test_identify_decal_material_flags_pom() {
    let (is_decal, is_pom) = identify_decal_material_flags("Shader { %PARALLAX %VERTEX_COLORS }");
    assert!(!is_decal);
    assert!(is_pom);
}

#[test]
fn test_identify_decal_material_flags_pom_alt() {
    let (is_decal, is_pom) = identify_decal_material_flags("Shader { %POM }");
    assert!(!is_decal);
    assert!(is_pom);
}

#[test]
fn test_identify_decal_material_flags_both() {
    let (is_decal, is_pom) = identify_decal_material_flags("Shader { %DECAL %POM }");
    assert!(is_decal);
    assert!(is_pom);
}

#[test]
fn test_identify_decal_material_flags_neither() {
    let (is_decal, is_pom) = identify_decal_material_flags("Shader { %VERTEX_COLORS %NORMAL_MAP }");
    assert!(!is_decal);
    assert!(!is_pom);
}

#[test]
fn test_identify_meshes_with_decals_single() {
    let mesh_materials = vec![
        (
            "Mesh_001".to_string(),
            vec![
                ("Base_Material".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                ("Decal_Glass".to_string(), "Shader { %DECAL }".to_string()),
                ("Metal".to_string(), "Shader { %METALLIC }".to_string()),
            ],
        ),
    ];
    
    let result = identify_meshes_with_decals(&mesh_materials).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].mesh_path, "Mesh_001");
    assert_eq!(result[0].decal_materials.len(), 1);
    assert_eq!(result[0].decal_materials[0].material_index, 1);
}

#[test]
fn test_identify_meshes_with_decals_multiple() {
    let mesh_materials = vec![
        (
            "Mesh_A".to_string(),
            vec![
                ("Decal_001".to_string(), "Shader { %DECAL }".to_string()),
                ("Normal_Mat".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
            ],
        ),
        (
            "Mesh_B".to_string(),
            vec![
                ("POM_Rock".to_string(), "Shader { %POM }".to_string()),
                ("Decal_002".to_string(), "Shader { %DECAL %PARALLAX }".to_string()),
            ],
        ),
        (
            "Mesh_C".to_string(),
            vec![
                ("Base".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                ("Diffuse".to_string(), "Shader { %NORMAL_MAP }".to_string()),
            ],
        ),
    ];
    
    let result = identify_meshes_with_decals(&mesh_materials).unwrap();
    assert_eq!(result.len(), 2);  // Only Mesh_A and Mesh_B have decals
    assert_eq!(result[0].decal_materials.len(), 1);
    assert_eq!(result[1].decal_materials.len(), 2);
}

#[test]
fn test_identify_meshes_with_decals_none() {
    let mesh_materials = vec![
        (
            "Mesh_001".to_string(),
            vec![
                ("Material_A".to_string(), "Shader { %VERTEX_COLORS }".to_string()),
                ("Material_B".to_string(), "Shader { %NORMAL_MAP }".to_string()),
            ],
        ),
    ];
    
    let result = identify_meshes_with_decals(&mesh_materials).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_validate_decal_material_identification_no_materials() {
    let meshes = vec![
        MeshWithDecals {
            mesh_path: "Mesh_001".to_string(),
            decal_materials: vec![],
            decal_face_indices: vec![],
        },
    ];
    
    assert!(validate_decal_material_identification(&meshes).is_err());
}
