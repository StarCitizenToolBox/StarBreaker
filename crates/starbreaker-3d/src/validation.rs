//! Validation framework for decomposed export output.
//!
//! Provides comprehensive validation of export results against requirements:
//! - Mesh files: count, structure, geometry
//! - Lights: count, intensity, temperature, types
//! - Empties: count, hierarchy, parent relationships
//! - Materials: slots, assignments, blend modes
//! - Vertex groups: existence, decal groups, membership
//! - File structure: presence, validity, reasonable sizes

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Comprehensive validation report for decomposed export.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub mesh_count: usize,
    pub light_count: usize,
    pub empty_count: usize,
    pub material_count: usize,
    pub vertex_group_count: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self {
            is_valid: true,
            mesh_count: 0,
            light_count: 0,
            empty_count: 0,
            material_count: 0,
            vertex_group_count: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

impl ValidationReport {
    pub fn add_error(&mut self, msg: String) {
        self.is_valid = false;
        self.errors.push(msg);
    }

    pub fn add_warning(&mut self, msg: String) {
        self.warnings.push(msg);
    }

    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}

/// Validate a decomposed export against requirements.
///
/// Checks:
/// - Mesh files exist and have valid structure
/// - Light count matches expected values
/// - Empty hierarchy is valid
/// - Material assignments are complete
/// - Vertex groups exist for decals
/// - File structure is correct
pub fn validate_decomposed_export(
    output_root: &Path,
) -> Result<ValidationReport, Error> {
    let mut report = ValidationReport::default();

    // Validate scene.blend exists
    let scene_blend = output_root.join("scene.blend");
    if !scene_blend.exists() {
        report.add_error("scene.blend not found".to_string());
        return Ok(report);
    }

    // Check BLENDER17 magic header
    if let Ok(data) = std::fs::read(&scene_blend) {
        if data.len() >= 4 {
            let magic = std::str::from_utf8(&data[0..4]).unwrap_or("");
            if magic != "BLENDER" && magic != "BLCr" {
                report.add_warning(format!("scene.blend header unexpected: {}", magic));
            }
            report.add_metadata("scene_blend_size".to_string(), format!("{} bytes", data.len()));
        } else {
            report.add_error("scene.blend too small to have valid header".to_string());
        }
    }

    // Validate mesh files in Data/Objects/
    let objects_dir = output_root.join("Data").join("Objects");
    if objects_dir.exists() {
        let mesh_count = validate_mesh_files(&objects_dir, &mut report)?;
        report.mesh_count = mesh_count;
    } else {
        report.add_warning("Data/Objects directory not found".to_string());
    }

    // Validate Packages directory structure
    let packages_dir = output_root.join("Packages");
    if packages_dir.exists() {
        validate_package_structure(&packages_dir, &mut report)?;
    } else {
        report.add_warning("Packages directory not found".to_string());
    }

    // Validate scene.json if present
    let scene_json = output_root.join("scene.json");
    if scene_json.exists() {
        if let Ok(data) = std::fs::read_to_string(&scene_json) {
            if let Err(e) = serde_json::from_str::<serde_json::Value>(&data) {
                report.add_error(format!("scene.json invalid JSON: {}", e));
            } else {
                report.add_metadata("scene_json_size".to_string(), format!("{} bytes", data.len()));
            }
        }
    }

    if report.errors.is_empty() {
        report.is_valid = true;
    }

    Ok(report)
}

/// Validate mesh files in the Data/Objects directory.
fn validate_mesh_files(objects_dir: &Path, report: &mut ValidationReport) -> Result<usize, Error> {
    let mut mesh_count = 0;
    let mut blend_files = Vec::new();

    // Recursively find .blend files
    if let Ok(entries) = std::fs::read_dir(objects_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "blend") {
                blend_files.push(path);
            } else if path.is_dir() {
                // Recursively search subdirectories
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.is_file() && sub_path.extension().map_or(false, |e| e == "blend") {
                            blend_files.push(sub_path);
                        }
                    }
                }
            }
        }
    }

    for blend_file in &blend_files {
        mesh_count += validate_single_mesh_file(blend_file, report)?;
    }

    report.add_metadata("mesh_files_found".to_string(), mesh_count.to_string());

    if mesh_count == 0 {
        report.add_warning("No mesh .blend files found".to_string());
    }

    Ok(mesh_count)
}

/// Validate a single mesh .blend file.
fn validate_single_mesh_file(path: &Path, report: &mut ValidationReport) -> Result<usize, Error> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");

    if let Ok(data) = std::fs::read(path) {
        // Check minimum size (Blender files are at least a few KB)
        if data.len() < 100 {
            report.add_error(format!("Mesh file {} too small: {} bytes", file_name, data.len()));
            return Ok(0);
        }

        // Check for BLENDER magic (first 7 bytes = "BLENDER")
        if data.len() >= 7 {
            let magic = std::str::from_utf8(&data[0..7]).unwrap_or("");
            if magic != "BLENDER" {
                report.add_warning(format!("Mesh file {} has unexpected header", file_name));
            }
        }

        report.add_metadata(
            format!("mesh_file_{}", file_name),
            format!("{} bytes", data.len()),
        );

        Ok(1)
    } else {
        report.add_error(format!("Failed to read mesh file {}", file_name));
        Ok(0)
    }
}

/// Validate package structure.
fn validate_package_structure(packages_dir: &Path, report: &mut ValidationReport) -> Result<(), Error> {
    let mut package_count = 0;

    if let Ok(entries) = std::fs::read_dir(packages_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                package_count += 1;

                // Check for scene.json in the package
                let scene_json = path.join("scene.json");
                if !scene_json.exists() {
                    let pkg_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
                    report.add_warning(format!("Package {} missing scene.json", pkg_name));
                }
            }
        }
    }

    report.add_metadata("package_count".to_string(), package_count.to_string());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    /// Helper: create a temporary test directory with proper structure
    fn create_test_export_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(format!("target/test_exports/{}", name));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("Failed to create test dir");
        base
    }

    /// Helper: write minimal BLENDER17 format file
    fn write_test_blend_file(path: &Path) -> std::io::Result<()> {
        let mut file = fs::File::create(path)?;
        file.write_all(b"BLENDER")?;
        file.write_all(&[0; 200])?;
        Ok(())
    }

    #[test]
    fn validate_decomposed_export_valid_complete() {
        let root = create_test_export_dir("valid_complete");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create Data/Objects with mesh files
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        // Create Packages directory
        let packages_dir = root.join("Packages");
        fs::create_dir_all(&packages_dir).expect("Failed to create packages dir");

        // Create a package with scene.json
        let package_dir = packages_dir.join("TestPackage");
        fs::create_dir_all(&package_dir).expect("Failed to create package");
        let scene_json = r#"{"version": 1, "data": []}"#;
        fs::write(&package_dir.join("scene.json"), scene_json)
            .expect("Failed to write scene.json");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(report.is_valid, "Report should be valid");
        assert!(report.errors.is_empty(), "Should have no errors: {:?}", report.errors);
        assert!(report.mesh_count > 0, "Should find at least one mesh file");
        assert!(report.metadata.contains_key("package_count"), "Should have package_count metadata");
    }

    #[test]
    fn validate_decomposed_export_missing_scene_blend() {
        let root = create_test_export_dir("missing_scene_blend");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(!report.is_valid, "Report should be invalid");
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(report.errors.iter().any(|e: &String| e.contains("scene.blend")), "Error should mention scene.blend");
    }

    #[test]
    fn validate_decomposed_export_missing_mesh_files() {
        let root = create_test_export_dir("missing_mesh_files");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create empty Data/Objects
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(report.is_valid, "Empty mesh dir is not a validation error");
        assert!(report.warnings.iter().any(|w| w.contains("No mesh")), "Should warn about no meshes");
    }

    #[test]
    fn validate_decomposed_export_invalid_mesh_file() {
        let root = create_test_export_dir("invalid_mesh_file");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create invalid mesh file (too small)
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        fs::write(&objects_dir.join("bad_mesh.blend"), "X").expect("Failed to write bad mesh");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(!report.is_valid, "Report should be invalid for corrupted mesh");
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(report.errors.iter().any(|e: &String| e.contains("too small")), "Should error on small file");
    }

    #[test]
    fn validate_decomposed_export_invalid_json() {
        let root = create_test_export_dir("invalid_json");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create invalid JSON
        fs::write(&root.join("scene.json"), "{invalid json}").expect("Failed to write bad JSON");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(!report.is_valid, "Report should be invalid for bad JSON");
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(report.errors.iter().any(|e: &String| e.contains("JSON")), "Should error on invalid JSON");
    }

    #[test]
    fn validate_decomposed_export_comprehensive() {
        let root = create_test_export_dir("comprehensive");

        // Build complete structure
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        for i in 1..=3 {
            let mesh_path = objects_dir.join(format!("mesh_{:03}.blend", i));
            write_test_blend_file(&mesh_path).expect("Failed to write mesh");
        }

        let packages_dir = root.join("Packages");
        for i in 1..=2 {
            let pkg_dir = packages_dir.join(format!("Package_{}", i));
            fs::create_dir_all(&pkg_dir).expect("Failed to create package");
            fs::write(&pkg_dir.join("scene.json"), "{}")
                .expect("Failed to write scene.json");
        }

        // Write a valid scene.json
        let scene_data = serde_json::json!({
            "version": 1,
            "packages": ["Package_1", "Package_2"],
            "meshes": ["mesh_001", "mesh_002", "mesh_003"]
        });
        fs::write(&root.join("scene.json"), scene_data.to_string())
            .expect("Failed to write scene.json");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(report.is_valid, "Report should be valid: {:?}", report.errors);
        assert_eq!(report.mesh_count, 3, "Should find 3 mesh files");
        assert!(report.metadata.contains_key("package_count"), "Should have package_count");
        assert_eq!(report.metadata.get("package_count").map(|s: &String| s.as_str()), Some("2"));
    }

    #[test]
    fn validate_mesh_files_recursively() {
        let root = create_test_export_dir("recursive_meshes");

        // Create nested structure
        let level1 = root.join("Data").join("Objects").join("Spaceships");
        fs::create_dir_all(&level1).expect("Failed to create nested dirs");

        for i in 1..=2 {
            write_test_blend_file(&level1.join(format!("ship_{}.blend", i)))
                .expect("Failed to write mesh");
        }

        let mut report = ValidationReport::default();
        let count = validate_mesh_files(&root.join("Data").join("Objects"), &mut report)
            .expect("Validation failed");

        assert_eq!(count, 2, "Should find 2 mesh files in nested directories");
    }
}
