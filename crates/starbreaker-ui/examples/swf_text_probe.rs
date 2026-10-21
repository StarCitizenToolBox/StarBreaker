use starbreaker_p4k::MappedP4k;
use std::collections::HashMap;
use std::io::Cursor;
use swf::Tag;

fn main() {
    let p4k_path = std::env::var("SC_DATA_P4K").expect("SC_DATA_P4K not set");
    let p4k = MappedP4k::open(std::path::Path::new(&p4k_path)).expect("open p4k");

    // Probe the SWF paths given on the command line, or a default set of the
    // shared BuildingBlocks/font SWFs when no paths are supplied.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let default_targets = [
        r"Data\UI\BuildingBlocks\assets\SWF\BuildingBlocks_root.swf".to_string(),
        r"Data\UI\BuildingBlocks\assets\SWF\Canvas.swf".to_string(),
        r"Data\UI\fonts\Shared\fonts_en.gfx".to_string(),
    ];
    let targets: Vec<String> = if args.is_empty() {
        default_targets.to_vec()
    } else {
        args
    };

    for target in &targets {
        let target = target.as_str();
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
        let r = parsed.header.stage_size();
        let sw = (r.x_max.get() - r.x_min.get()) as f32 / 20.0;
        let sh = (r.y_max.get() - r.y_min.get()) as f32 / 20.0;
        println!(
            "version={} tags={} stage={:.1}x{:.1} aspect_h_over_w={:.3}",
            parsed.header.version(),
            parsed.tags.len(),
            sw,
            sh,
            if sw > 0.0 { sh / sw } else { 0.0 }
        );

        // Frame structure: count main-timeline frames, list frame labels, and
        // per-frame placed character ids (to see whether states live in
        // separate frames).
        {
            let mut frame: u32 = 0;
            let mut labels: Vec<(u32, String)> = Vec::new();
            let mut per_frame_places: Vec<(u32, Vec<u16>)> = Vec::new();
            let mut cur: Vec<u16> = Vec::new();
            for tag in &parsed.tags {
                match tag {
                    Tag::FrameLabel(fl) => {
                        labels.push((frame, fl.label.to_string_lossy(swf::UTF_8)))
                    }
                    Tag::PlaceObject(po) => {
                        if let swf::PlaceObjectAction::Place(id)
                        | swf::PlaceObjectAction::Replace(id) = po.action
                        {
                            cur.push(id);
                        }
                    }
                    Tag::ShowFrame => {
                        per_frame_places.push((frame, std::mem::take(&mut cur)));
                        frame += 1;
                    }
                    _ => {}
                }
            }
            println!("main-timeline frames={frame} labels={labels:?}");
            for (f, ids) in per_frame_places.iter().take(12) {
                println!("  frame {f}: placed_ids={ids:?}");
            }
        }

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

        // DefineSprite tree: each sprite's placed children (id, depth, name) and
        // nested text/sprite characters — reveals AS-driven state structure.
        for tag in &parsed.tags {
            if let Tag::DefineSprite(sprite) = tag {
                let mut kids: Vec<String> = Vec::new();
                let mut texts = 0usize;
                let mut edits = 0usize;
                for st in &sprite.tags {
                    match st {
                        Tag::PlaceObject(po) => {
                            let id = match po.action {
                                swf::PlaceObjectAction::Place(id)
                                | swf::PlaceObjectAction::Replace(id) => Some(id),
                                swf::PlaceObjectAction::Modify => None,
                            };
                            let name = po
                                .name
                                .as_ref()
                                .map(|n| n.to_string_lossy(swf::UTF_8))
                                .unwrap_or_default();
                            kids.push(format!("(id={id:?} depth={} name='{name}')", po.depth));
                        }
                        Tag::DefineText(_) | Tag::DefineText2(_) => texts += 1,
                        Tag::DefineEditText(_) => edits += 1,
                        _ => {}
                    }
                }
                println!(
                    "sprite id={} frames={} places={} inner_text={} inner_edit={}: {}",
                    sprite.id,
                    sprite.num_frames,
                    kids.len(),
                    texts,
                    edits,
                    kids.join(" ")
                );
            }
        }

        // Static DefineText: the font(s) it uses (the no-target "NO TARGET" may be
        // a static text in Furore).
        for tag in &parsed.tags {
            if let Tag::DefineText(text) = tag {
                let fonts: Vec<u16> = text
                    .records
                    .iter()
                    .filter_map(|r| r.font_id)
                    .collect();
                let glyphs: usize = text.records.iter().map(|r| r.glyphs.len()).sum();
                println!(
                    "define_text id={} font_ids={:?} glyphs={}",
                    text.id, fonts, glyphs
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
                let initial_text = edit
                    .initial_text()
                    .map(|s| s.to_string_lossy(swf::UTF_8))
                    .unwrap_or_default();
                let height_px = edit.height().map(|twips| twips.get() as f32 / 20.0);
                let bounds = edit.bounds();
                let w = (bounds.x_max.get() - bounds.x_min.get()) as f32 / 20.0;
                let h = (bounds.y_max.get() - bounds.y_min.get()) as f32 / 20.0;
                println!(
                    "id={} var={} initial_text={:?} font_id={:?} font_name={:?} font_class={:?} height_px={:?} auto_size={} bounds=({:.1},{:.1})-({:.1},{:.1}) size=({:.1}x{:.1})",
                    edit.id(),
                    variable_name,
                    initial_text,
                    edit.font_id(),
                    edit.font_id().and_then(|id| font_names.get(&id).cloned()),
                    edit.font_class().map(|s| s.to_string_lossy(swf::UTF_8)),
                    height_px,
                    edit.is_auto_size(),
                    bounds.x_min.get() as f32 / 20.0,
                    bounds.y_min.get() as f32 / 20.0,
                    bounds.x_max.get() as f32 / 20.0,
                    bounds.y_max.get() as f32 / 20.0,
                    w,
                    h
                );
            }
        }

        println!("define_edit_text tags: {edit_text_count}");
    }
}
