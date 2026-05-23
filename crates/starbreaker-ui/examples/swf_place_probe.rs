use starbreaker_p4k::MappedP4k;
use std::path::Path;
use swf::CharacterId;

use starbreaker_ui::swf_assets::SwfAssetLibrary;

fn dump_sprite(lib: &SwfAssetLibrary, sprite_id: CharacterId, indent: usize, max_depth: usize) {
    if indent > max_depth {
        return;
    }

    for record in lib.extract_sprite_first_frame(sprite_id) {
        println!(
            "{space}depth={} char_id={} name={:?} export={:?}",
            record.depth,
            record.character_id,
            record.name,
            lib.export_name_for(record.character_id),
            space = " ".repeat(indent * 2)
        );
        dump_sprite(lib, record.character_id, indent + 1, max_depth);
    }
}

fn main() {
    let p4k_path = std::env::var("SC_DATA_P4K").expect("SC_DATA_P4K not set");
    let p4k = MappedP4k::open(Path::new(&p4k_path)).expect("open p4k");

    let target = r"Data\UI\BuildingBlocks\assets\SWF\Canvas.swf";
    let entry = p4k
        .entries()
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(target))
        .expect("Canvas.swf not found");

    let bytes = p4k.read(entry).expect("read swf");
    let lib = SwfAssetLibrary::new(bytes).expect("parse swf");

    println!("== stage frame 0 ==");
    for record in lib.stage_frame(0) {
        println!(
            "depth={} char_id={} name={:?} export={:?}",
            record.depth,
            record.character_id,
            record.name,
            lib.export_name_for(record.character_id)
        );
        dump_sprite(&lib, record.character_id, 1, 3);
    }

    println!("== exported sprites first frame ==");
    let mut export_names: Vec<_> = [
        "$Text1Book",
        "$Text1Bold",
        "$Text1Med",
        "$OutfitRegular",
        "$CIGDrake",
    ]
    .into_iter()
    .filter_map(|name| lib.lookup_export(name).map(|id| (name, id)))
    .collect();
    export_names.sort_by_key(|(_, id)| *id);

    for (name, id) in export_names {
        println!("export {} -> {}", name, id);
        for record in lib.extract_sprite_first_frame(id) {
            println!(
                "  depth={} char_id={} name={:?} export={:?}",
                record.depth,
                record.character_id,
                record.name,
                lib.export_name_for(record.character_id)
            );
        }
    }
}