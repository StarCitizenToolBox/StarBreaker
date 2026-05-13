#[test]
#[ignore]
fn check_gobo_dds_format() {
    use starbreaker_dds::DdsFile;
    use starbreaker_p4k::MappedP4k;
    
    let p4k_path = "/home/tom/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k";
    let p4k = MappedP4k::open(p4k_path).expect("Failed to open P4k");
    
    println!("\n=== Searching for gobo DDS files ===");
    
    // Try various paths to find a gobo
    let possible_paths = vec![
        "Data/Textures/lights/generic/spot_075_TEX0.dds",
        "data/textures/lights/generic/spot_075_tex0.dds",
        "Data\\Textures\\lights\\generic\\spot_075_TEX0.dds",
    ];
    
    for path in possible_paths {
        match p4k.entry_case_insensitive(path) {
            Some(entry) => {
                match p4k.read(&entry) {
                    Ok(bytes) => {
                        match DdsFile::from_bytes(&bytes) {
                            Ok(dds) => {
                                println!("✓ Found and parsed: {}", path);
                                
                                // Try to decode as BC6H
                                let result = dds.decode_bc6h_to_float_rgb(0);
                                
                                match result {
                                    Ok(Some((w, h, _))) => {
                                        println!("  ✓ BC6H detected: {}x{}", w, h);
                                        println!("    → Will export as EXR with full HDR support");
                                    }
                                    Ok(None) => {
                                        println!("  ✗ Not BC6H format");
                                        println!("    → Source DDS is likely BC1, BC3, BC7, or uncompressed");
                                        println!("    → Will export as PNG (8-bit LDR)");
                                    }
                                    Err(e) => println!("  ✗ Error decoding: {}", e),
                                }
                                return;
                            }
                            Err(e) => {
                                println!("  ✗ Failed to parse DDS: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("  ✗ Failed to read: {}", e);
                    }
                }
            }
            None => {
                println!("  ✗ Not found: {}", path);
            }
        }
    }
    
    panic!("Could not find any gobo DDS file");
}
