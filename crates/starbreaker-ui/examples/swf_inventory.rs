use starbreaker_p4k::MappedP4k;
use swf::Tag;
use std::io::Cursor;

fn main() {
    let p4k_path = std::env::var("SC_DATA_P4K").expect("SC_DATA_P4K not set");
    let p4k = MappedP4k::open(std::path::Path::new(&p4k_path)).expect("open p4k");
    let mut swf_entries: Vec<_> = p4k.entries().iter()
        .filter(|e| e.name.to_lowercase().ends_with(".swf"))
        .collect();
    swf_entries.sort_by_key(|e| e.name.clone());
    println!("Found {} SWFs", swf_entries.len());
    let mut chrome_swfs = Vec::new();
    for entry in &swf_entries {
        let Ok(bytes) = p4k.read(entry) else { continue };
        let Ok(swf_buf) = swf::decompress_swf(Cursor::new(&bytes[..])) else { continue };
        let Ok(parsed) = swf::parse_swf(&swf_buf) else { continue };
        let mut shape_count = 0usize;
        let mut export_count = 0usize;
        let mut exported_names = Vec::new();
        for tag in &parsed.tags {
            match tag {
                Tag::DefineShape(_) => shape_count += 1,
                Tag::ExportAssets(exports) => {
                    export_count += exports.len();
                    for asset in exports {
                        let n = asset.name.to_string_lossy(swf::UTF_8);
                        if n.to_lowercase().contains("shape_") || n.to_lowercase().contains("chevron") || n.to_lowercase().contains("greeble") || n.to_lowercase().contains("annunc") || n.to_lowercase().contains("notarget") {
                            exported_names.push(n);
                        }
                    }
                }
                _ => {}
            }
        }
        if shape_count > 0 || !exported_names.is_empty() {
            println!("{}: shapes={} exports={} interesting={:?}", entry.name, shape_count, export_count, exported_names.iter().take(8).collect::<Vec<_>>());
            chrome_swfs.push((entry.name.clone(), shape_count, exported_names.len()));
        }
    }
    println!("---");
    println!("{} SWFs with shapes or interesting exports", chrome_swfs.len());
}
