use starbreaker_p4k::MappedP4k;
use std::collections::HashMap;
use std::io::Cursor;
use swf::Tag;

fn main() {
    let p4k_path = std::env::var("SC_DATA_P4K").expect("SC_DATA_P4K not set");
    let p4k = MappedP4k::open(std::path::Path::new(&p4k_path)).expect("open p4k");

    let targets = [
        r"Data\UI\BuildingBlocks\assets\SWF\BuildingBlocks_root.swf",
        r"Data\UI\BuildingBlocks\assets\SWF\Canvas.swf",
        r"Data\UI\fonts\Shared\fonts_en.gfx",
    ];

    for target in targets {
        let Some(entry) = p4k
            .entries()
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(target))
        else {
            println!("missing: {target}");
            continue;
        };

        let bytes = p4k.read(entry).expect("read swf");
        let swf_buf = swf::decompress_swf(Cursor::new(&bytes[..])).expect("decompress");
        let parsed = swf::parse_swf(&swf_buf).expect("parse");

        println!("=== {} ===", entry.name);
        println!("version={} tags={}", parsed.header.version(), parsed.tags.len());

        let mut font_names: HashMap<u16, String> = HashMap::new();
        let mut font_defs = 0usize;
        let mut cff_font_defs = 0usize;
        for tag in &parsed.tags {
            match tag {
                Tag::DefineFont(font) => {
                    font_defs += 1;
                    font_names.insert(font.id, format!("<DefineFont:{}>", font.id));
                }
                Tag::DefineFont2(font) => {
                    font_defs += 1;
                    font_names.insert(
                        font.id,
                        format!(
                            "{}{}{}",
                            font.name.to_string_lossy(swf::UTF_8),
                            if font.flags.contains(swf::FontFlag::IS_BOLD) {
                                " [bold]"
                            } else {
                                ""
                            },
                            if font.flags.contains(swf::FontFlag::IS_ITALIC) {
                                " [italic]"
                            } else {
                                ""
                            }
                        ),
                    );
                }
                Tag::DefineFont4(font4) => {
                    cff_font_defs += 1;
                    font_names.insert(
                        font4.id,
                        format!(
                            "{}{}{} [cff]",
                            font4.name.to_string_lossy(swf::UTF_8),
                            if font4.is_bold { " [bold]" } else { "" },
                            if font4.is_italic { " [italic]" } else { "" }
                        ),
                    );
                }
                _ => {}
            }
        }
        if font_defs > 0 || cff_font_defs > 0 {
            println!("font defs: {} (cff={})", font_defs, cff_font_defs);
            let mut ids: Vec<u16> = font_names.keys().copied().collect();
            ids.sort_unstable();
            for id in ids {
                if let Some(name) = font_names.get(&id) {
                    println!("  font id={} name={}", id, name);
                }
            }
        }

        for tag in &parsed.tags {
            if let Tag::DefineBinaryData(data) = tag {
                let bytes = data.data;
                let sig = if bytes.len() >= 4 {
                    format!(
                        "{:02X} {:02X} {:02X} {:02X}",
                        bytes[0], bytes[1], bytes[2], bytes[3]
                    )
                } else {
                    String::from("<short>")
                };
                println!(
                    "binary_data id={} size={} sig={} ",
                    data.id,
                    bytes.len(),
                    sig
                );
            }
        }

        for tag in &parsed.tags {
            if let Tag::ImportAssets { url, imports } = tag {
                println!(
                    "import_assets url={} symbols={}",
                    url.to_string_lossy(swf::UTF_8),
                    imports.len()
                );
                for import in imports {
                    println!(
                        "  import id={} name={}",
                        import.id,
                        import.name.to_string_lossy(swf::UTF_8)
                    );
                }
            }
        }

        let mut exports_all = Vec::new();
        let mut style_exports = Vec::new();
        for tag in &parsed.tags {
            if let Tag::ExportAssets(exports) = tag {
                for export in exports {
                    let name = export.name.to_string_lossy(swf::UTF_8);
                    exports_all.push((export.id, name.clone()));
                    let lower = name.to_ascii_lowercase();
                    if lower.contains("heading")
                        || lower.contains("caption")
                        || lower.contains("body")
                        || lower.contains("textfield")
                    {
                        style_exports.push((export.id, name));
                    }
                }
            }
        }
        if !exports_all.is_empty() {
            println!("exports: {}", exports_all.len());
            for (id, name) in exports_all.iter().take(200) {
                println!("  export id={} name={}", id, name);
            }
        }
        if !style_exports.is_empty() {
            println!("style-like exports: {}", style_exports.len());
            for (id, name) in style_exports.iter().take(80) {
                println!("  export id={} name={}", id, name);
            }
        }

        let mut edit_text_count = 0usize;
        for tag in &parsed.tags {
            if let Tag::DefineEditText(edit) = tag {
                edit_text_count += 1;
                let variable_name = edit.variable_name().to_string_lossy(swf::UTF_8);
                let height_px = edit.height().map(|twips| twips.get() as f32 / 20.0);
                let bounds = edit.bounds();
                let w = (bounds.x_max.get() - bounds.x_min.get()) as f32 / 20.0;
                let h = (bounds.y_max.get() - bounds.y_min.get()) as f32 / 20.0;
                println!(
                    "id={} var={} font_id={:?} font_name={:?} font_class={:?} height_px={:?} auto_size={} bounds=({:.1}x{:.1})",
                    edit.id(),
                    variable_name,
                    edit.font_id(),
                    edit.font_id().and_then(|id| font_names.get(&id).cloned()),
                    edit.font_class().map(|s| s.to_string_lossy(swf::UTF_8)),
                    height_px,
                    edit.is_auto_size(),
                    w,
                    h
                );
            }
        }

        println!("define_edit_text tags: {edit_text_count}");
    }
}
