//! Phase 4B tests for decal vertex group creation.

use super::*;

#[test]
fn test_map_faces_to_vertices_single_triangle() {
    let corner_verts = vec![0, 1, 2, 3, 4, 5];  // Two triangles
    let face_indices = vec![0];  // First triangle
    let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
    
    assert_eq!(vertices, vec![0, 1, 2]);
}

#[test]
fn test_map_faces_to_vertices_multiple_faces() {
    let corner_verts = vec![0, 1, 2, 2, 3, 4, 4, 5, 6];  // Three triangles
    let face_indices = vec![0, 2];  // First and third triangles
    let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
    
    assert_eq!(vertices, vec![0, 1, 2, 4, 5, 6]);
}

#[test]
fn test_map_faces_to_vertices_shared_vertices() {
    let corner_verts = vec![0, 1, 2, 1, 2, 3];  // Two triangles sharing edge 1-2
    let face_indices = vec![0, 1];
    let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
    
    assert_eq!(vertices, vec![0, 1, 2, 3]);
}

#[test]
fn test_map_faces_to_vertices_empty() {
    let corner_verts = vec![0, 1, 2];
    let face_indices = vec![];
    let vertices = map_faces_to_vertices(&face_indices, &corner_verts, 3).unwrap();
    
    assert_eq!(vertices.len(), 0);
}

#[test]
fn test_map_faces_to_vertices_out_of_bounds() {
    let corner_verts = vec![0, 1, 2];  // Only one triangle
    let face_indices = vec![1];  // Second triangle (doesn't exist)
    let result = map_faces_to_vertices(&face_indices, &corner_verts, 3);
    
    assert!(result.is_err());
}

#[test]
fn test_collect_decal_vertices_single_face() {
    let mesh_with_decals = MeshWithDecals {
        mesh_path: "Mesh_001".to_string(),
        decal_materials: vec![
            DecalMaterial {
                material_name: "Decal_001".to_string(),
                is_decal: true,
                is_pom: false,
                material_index: 0,
            },
        ],
        decal_face_indices: vec![0],
    };
    
    let corner_verts = vec![0, 1, 2, 3, 4, 5];  // Two triangles
    let face_indices = vec![0];  // First triangle
    
    let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
    assert_eq!(result.mesh_name, "Mesh_001");
    assert_eq!(result.total_vertices, 6);
    assert_eq!(result.vertex_groups.len(), 1);
    assert_eq!(result.vertex_groups[0].name, "starbreaker_decal_offset");
    assert_eq!(result.vertex_groups[0].vertex_indices, vec![0, 1, 2]);
}

#[test]
fn test_collect_decal_vertices_multiple_faces() {
    let mesh_with_decals = MeshWithDecals {
        mesh_path: "Mesh_002".to_string(),
        decal_materials: vec![
            DecalMaterial {
                material_name: "Decal_001".to_string(),
                is_decal: true,
                is_pom: false,
                material_index: 0,
            },
            DecalMaterial {
                material_name: "POM_001".to_string(),
                is_decal: false,
                is_pom: true,
                material_index: 2,
            },
        ],
        decal_face_indices: vec![],
    };
    
    let corner_verts = vec![
        0, 1, 2,      // Face 0
        2, 3, 4,      // Face 1 (shares edge with Face 0)
        4, 5, 6,      // Face 2
    ];
    let face_indices = vec![0, 2];  // First and third faces
    
    let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
    assert_eq!(result.vertex_groups[0].name, "starbreaker_decal_offset");
    // Should contain vertices from faces 0 and 2: {0, 1, 2, 4, 5, 6}
    assert_eq!(result.vertex_groups[0].vertex_indices.len(), 6);
}

#[test]
fn test_collect_decal_vertices_empty_faces() {
    let mesh_with_decals = MeshWithDecals {
        mesh_path: "Mesh_003".to_string(),
        decal_materials: vec![],
        decal_face_indices: vec![],
    };
    
    let corner_verts = vec![0, 1, 2, 3, 4, 5];
    let face_indices = vec![];
    
    let result = collect_decal_vertices(&mesh_with_decals, &face_indices, &corner_verts, 3).unwrap();
    assert_eq!(result.vertex_groups[0].name, "starbreaker_decal_offset");
    assert_eq!(result.vertex_groups[0].vertex_indices.len(), 0);
}

#[test]
fn test_validate_vertex_groups_valid() {
    let vgroups = vec![
        VertexGroup {
            name: "starbreaker_decal_offset".to_string(),
            vertex_indices: vec![0, 1, 2, 3],
        },
    ];
    
    assert!(validate_vertex_groups(&vgroups, 10).is_ok());
}

#[test]
fn test_validate_vertex_groups_empty_name() {
    let vgroups = vec![
        VertexGroup {
            name: "".to_string(),
            vertex_indices: vec![0, 1],
        },
    ];
    
    assert!(validate_vertex_groups(&vgroups, 10).is_err());
}

#[test]
fn test_validate_vertex_groups_out_of_bounds() {
    let vgroups = vec![
        VertexGroup {
            name: "Group".to_string(),
            vertex_indices: vec![0, 1, 15],  // 15 is out of bounds
        },
    ];
    
    assert!(validate_vertex_groups(&vgroups, 10).is_err());
}
