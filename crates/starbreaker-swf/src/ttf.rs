//! Convert a parsed SWF `Font` into a valid TrueType `.ttf` byte blob.
//!
//! SWF DefineFont3 stores glyph outlines as quadratic Bézier shape records in
//! Twips (1/20 px), with an implicit em-size of 1024 px = 20480 twips. TTF
//! supports quadratic Béziers natively, so the conversion is direct: walk the
//! shape records into a kurbo `BezPath`, scale to UPEM=2048 (i.e. divide by
//! 10) and flip Y (SWF down → TTF up), then hand off to
//! `write_fonts::tables::glyf::SimpleGlyph`.
//!
//! Tables emitted: head, hhea, maxp, OS/2, name, cmap (format 4 BMP), glyf,
//! loca, hmtx, post (v3 — no glyph names). This is the minimum that produces
//! a font viewable in Windows + macOS + FreeType-based renderers.

use crate::SwfError;
use crate::shape::{shape_records_to_bezpath, transform_path};
use swf::{Font, FontFlag};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{GlyfLocaBuilder, SimpleGlyph};
use write_fonts::tables::head::{Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::tables::post::Post;
use write_fonts::types::{FWord, Fixed, GlyphId, LongDateTime, Tag, UfWord, Version16Dot16};

/// Convert a single SWF `Font` (DefineFont2 / DefineFont3) into TrueType bytes.
pub fn font_to_ttf(font: &Font) -> Result<Vec<u8>, SwfError> {
    // DefineFont3 stores coordinates as twips with implicit em = 20480.
    // DefineFont2 stores coordinates as font-units with em = 1024 but the
    // swf crate still wraps them in `Twips`, so the same scale factor of
    // 1/10 maps both to UPEM=2048. (For DefineFont2 fonts you'd want UPEM=
    // 1024 with scale 1/20; for now we only have v3 fonts in SC.)
    const UPEM: u16 = 2048;
    let scale = 1.0_f64 / 10.0;

    // ── Build glyph 0 (.notdef) as an empty glyph ───────────────────────
    let notdef = SimpleGlyph::default();
    let mut simple_glyphs: Vec<SimpleGlyph> = Vec::with_capacity(font.glyphs.len() + 1);
    let mut advances: Vec<u16> = Vec::with_capacity(font.glyphs.len() + 1);
    let mut left_side_bearings: Vec<i16> = Vec::with_capacity(font.glyphs.len() + 1);
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();

    simple_glyphs.push(notdef);
    advances.push((UPEM / 2) as u16);
    left_side_bearings.push(0);

    let mut overall_x_min: i16 = 0;
    let mut overall_y_min: i16 = 0;
    let mut overall_x_max: i16 = 0;
    let mut overall_y_max: i16 = 0;
    let mut min_lsb: i16 = 0;
    let mut max_extent: i16 = 0;
    let mut max_advance: u16 = (UPEM / 2) as u16;
    let mut max_points_per_glyph: u16 = 0;
    let mut max_contours_per_glyph: u16 = 0;
    let mut total_advance_for_avg: i64 = 0;
    let mut non_zero_advance_count: i64 = 0;
    let mut first_char: u16 = u16::MAX;
    let mut last_char: u16 = 0;

    // ── Per-glyph conversion ────────────────────────────────────────────
    for (i, glyph) in font.glyphs.iter().enumerate() {
        let glyph_id = (i + 1) as u16; // 0 is .notdef
        let raw_path = shape_records_to_bezpath(&glyph.shape_records);
        // Scale: twips/10 → em-units. Y-flip: SWF down → TTF up.
        let path = transform_path(&raw_path, scale, -scale, 0.0, 0.0);

        let simple = if path.elements().is_empty() {
            SimpleGlyph::default()
        } else {
            SimpleGlyph::from_bezpath(&path)
                .map_err(|e| SwfError::Tags(format!("glyph {glyph_id} from_bezpath: {e:?}")))?
        };

        // Accumulate stats
        let bb = simple.bbox;
        if !path.elements().is_empty() {
            overall_x_min = overall_x_min.min(bb.x_min);
            overall_y_min = overall_y_min.min(bb.y_min);
            overall_x_max = overall_x_max.max(bb.x_max);
            overall_y_max = overall_y_max.max(bb.y_max);
            min_lsb = min_lsb.min(bb.x_min);
            max_extent = max_extent.max(bb.x_max);
            let points: usize = simple.contours.iter().map(|c| c.len()).sum();
            max_points_per_glyph = max_points_per_glyph.max(points.min(u16::MAX as usize) as u16);
            max_contours_per_glyph =
                max_contours_per_glyph.max(simple.contours.len().min(u16::MAX as usize) as u16);
        }

        // Advance: SWF stores in font units (DefineFont3 has it × 1 = same scale as shape coords).
        let advance_swf = glyph.advance as f64;
        let advance = (advance_swf * scale).round().max(0.0) as u16;
        max_advance = max_advance.max(advance);
        if advance > 0 {
            total_advance_for_avg += advance as i64;
            non_zero_advance_count += 1;
        }

        simple_glyphs.push(simple);
        advances.push(advance);
        left_side_bearings.push(bb.x_min);

        // Codepoint mapping
        if glyph.code != 0
            && let Some(ch) = char::from_u32(glyph.code as u32)
        {
            mappings.push((ch, GlyphId::new(glyph_id.into())));
            first_char = first_char.min(glyph.code);
            last_char = last_char.max(glyph.code);
        }
    }

    if first_char == u16::MAX {
        first_char = 0;
    }

    // ── glyf + loca ────────────────────────────────────────────────────
    let mut glyf_builder = GlyfLocaBuilder::new();
    for sg in &simple_glyphs {
        glyf_builder
            .add_glyph(sg)
            .map_err(|e| SwfError::Tags(format!("add_glyph: {e}")))?;
    }
    let (glyf, loca, loca_format) = glyf_builder.build();

    // ── cmap ───────────────────────────────────────────────────────────
    let cmap = Cmap::from_mappings(mappings.iter().copied())
        .map_err(|e| SwfError::Tags(format!("cmap: {e:?}")))?;

    // ── hmtx ───────────────────────────────────────────────────────────
    let h_metrics: Vec<LongMetric> = advances
        .iter()
        .zip(left_side_bearings.iter())
        .map(|(&adv, &lsb)| LongMetric::new(adv, lsb))
        .collect();
    let hmtx = Hmtx::new(h_metrics, vec![]);

    // ── maxp ───────────────────────────────────────────────────────────
    let mut maxp = Maxp {
        num_glyphs: simple_glyphs.len() as u16,
        ..Default::default()
    };
    maxp.max_points = Some(max_points_per_glyph);
    maxp.max_contours = Some(max_contours_per_glyph);
    maxp.max_composite_points = Some(0);
    maxp.max_composite_contours = Some(0);
    maxp.max_zones = Some(2);
    maxp.max_twilight_points = Some(0);
    maxp.max_storage = Some(0);
    maxp.max_function_defs = Some(0);
    maxp.max_instruction_defs = Some(0);
    maxp.max_stack_elements = Some(0);
    maxp.max_size_of_instructions = Some(0);
    maxp.max_component_elements = Some(0);
    maxp.max_component_depth = Some(0);

    // ── Vertical metrics ──────────────────────────────────────────────
    let (ascent, descent, line_gap) = if let Some(layout) = &font.layout {
        let asc = (layout.ascent as f64 * scale).round() as i16;
        let desc = -(layout.descent as f64 * scale).round() as i16;
        let gap = (layout.leading as f64 * scale).round() as i16;
        (asc, desc, gap)
    } else {
        // Reasonable defaults if no layout
        let asc = (UPEM as i16 * 7) / 10;
        let desc = -((UPEM as i16 * 2) / 10);
        (asc, desc, 0)
    };

    // ── head ───────────────────────────────────────────────────────────
    let mac_style = {
        let mut s = MacStyle::empty();
        if font.flags.contains(FontFlag::IS_BOLD) {
            s |= MacStyle::BOLD;
        }
        if font.flags.contains(FontFlag::IS_ITALIC) {
            s |= MacStyle::ITALIC;
        }
        s
    };

    let head = Head {
        font_revision: Fixed::from_f64(1.0),
        flags: Default::default(),
        units_per_em: UPEM,
        created: LongDateTime::new(0),
        modified: LongDateTime::new(0),
        x_min: overall_x_min,
        y_min: overall_y_min,
        x_max: overall_x_max,
        y_max: overall_y_max,
        mac_style,
        lowest_rec_ppem: 7,
        font_direction_hint: 2,
        index_to_loc_format: match loca_format {
            write_fonts::tables::loca::LocaFormat::Short => 0,
            write_fonts::tables::loca::LocaFormat::Long => 1,
        },
        checksum_adjustment: 0,
        magic_number: 0x5F0F3CF5,
    };

    // ── hhea ───────────────────────────────────────────────────────────
    let hhea = Hhea {
        ascender: FWord::new(ascent),
        descender: FWord::new(descent),
        line_gap: FWord::new(line_gap),
        advance_width_max: UfWord::new(max_advance),
        min_left_side_bearing: FWord::new(min_lsb),
        min_right_side_bearing: FWord::new(0),
        x_max_extent: FWord::new(max_extent),
        caret_slope_rise: 1,
        caret_slope_run: 0,
        caret_offset: 0,
        number_of_h_metrics: advances.len() as u16,
    };

    // ── OS/2 ───────────────────────────────────────────────────────────
    let avg = if non_zero_advance_count > 0 {
        (total_advance_for_avg / non_zero_advance_count) as i16
    } else {
        (UPEM / 2) as i16
    };
    let weight = if font.flags.contains(FontFlag::IS_BOLD) {
        700
    } else {
        400
    };
    let mut fs_selection = SelectionFlags::empty();
    if font.flags.contains(FontFlag::IS_BOLD) {
        fs_selection |= SelectionFlags::BOLD;
    }
    if font.flags.contains(FontFlag::IS_ITALIC) {
        fs_selection |= SelectionFlags::ITALIC;
    }
    if fs_selection.is_empty() {
        fs_selection |= SelectionFlags::REGULAR;
    }

    let strikeout_size = (UPEM as f64 * 0.05).round() as i16;
    let strikeout_pos = (UPEM as f64 * 0.25).round() as i16;
    let os2 = Os2 {
        x_avg_char_width: avg,
        us_weight_class: weight,
        us_width_class: 5,
        fs_type: 0,
        y_subscript_x_size: (UPEM as f64 * 0.65) as i16,
        y_subscript_y_size: (UPEM as f64 * 0.6) as i16,
        y_subscript_x_offset: 0,
        y_subscript_y_offset: (UPEM as f64 * 0.075) as i16,
        y_superscript_x_size: (UPEM as f64 * 0.65) as i16,
        y_superscript_y_size: (UPEM as f64 * 0.6) as i16,
        y_superscript_x_offset: 0,
        y_superscript_y_offset: (UPEM as f64 * 0.35) as i16,
        y_strikeout_size: strikeout_size,
        y_strikeout_position: strikeout_pos,
        s_family_class: 0,
        panose_10: [0; 10],
        ul_unicode_range_1: 1, // Bit 0: Basic Latin
        ul_unicode_range_2: 0,
        ul_unicode_range_3: 0,
        ul_unicode_range_4: 0,
        ach_vend_id: Tag::new(b"NONE"),
        fs_selection,
        us_first_char_index: first_char,
        us_last_char_index: last_char,
        s_typo_ascender: ascent,
        s_typo_descender: descent,
        s_typo_line_gap: line_gap,
        us_win_ascent: ascent.max(0) as u16,
        us_win_descent: (-descent).max(0) as u16,
        ul_code_page_range_1: Some(1), // Bit 0: Latin 1
        ul_code_page_range_2: Some(0),
        sx_height: None,
        s_cap_height: None,
        us_default_char: None,
        us_break_char: None,
        us_max_context: None,
        us_lower_optical_point_size: None,
        us_upper_optical_point_size: None,
    };

    // ── name ───────────────────────────────────────────────────────────
    let family = font.name.to_string_lossy(swf::UTF_8);
    let subfamily = if font.flags.contains(FontFlag::IS_BOLD) && font.flags.contains(FontFlag::IS_ITALIC) {
        "Bold Italic"
    } else if font.flags.contains(FontFlag::IS_BOLD) {
        "Bold"
    } else if font.flags.contains(FontFlag::IS_ITALIC) {
        "Italic"
    } else {
        "Regular"
    };
    let full_name = format!("{family} {subfamily}");
    // PostScript names: ASCII, no spaces, max 63 chars
    let postscript_name: String = full_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let postscript_name = if postscript_name.len() > 63 {
        postscript_name[..63].to_string()
    } else if postscript_name.is_empty() {
        "Unknown".to_string()
    } else {
        postscript_name
    };

    let name = Name::new(vec![
        // Platform 3 (Windows), Encoding 1 (Unicode BMP), Lang 0x409 (en-US)
        NameRecord::new(3, 1, 0x409, 1u16.into(), family.clone().into()),       // Family (id 1)
        NameRecord::new(3, 1, 0x409, 2u16.into(), subfamily.to_string().into()), // Subfamily (id 2)
        NameRecord::new(3, 1, 0x409, 3u16.into(), full_name.clone().into()),    // Unique ID (id 3, reuse full name)
        NameRecord::new(3, 1, 0x409, 4u16.into(), full_name.clone().into()),    // Full name (id 4)
        NameRecord::new(3, 1, 0x409, 5u16.into(), "Version 1.0".to_string().into()), // Version (id 5)
        NameRecord::new(3, 1, 0x409, 6u16.into(), postscript_name.into()),      // PostScript name (id 6)
    ]);

    // ── post ───────────────────────────────────────────────────────────
    let post = Post {
        version: Version16Dot16::new(3, 0),
        italic_angle: Fixed::from_f64(0.0),
        underline_position: FWord::new(-(UPEM as i16 / 10)),
        underline_thickness: FWord::new(UPEM as i16 / 20),
        is_fixed_pitch: 0,
        min_mem_type42: 0,
        max_mem_type42: 0,
        min_mem_type1: 0,
        max_mem_type1: 0,
        num_glyphs: Some(simple_glyphs.len() as u16),
        glyph_name_index: None,
        string_data: None,
    };

    // ── Assemble ───────────────────────────────────────────────────────
    let mut builder = FontBuilder::new();
    builder
        .add_table(&head)
        .map_err(|e| SwfError::Tags(format!("head: {e:?}")))?
        .add_table(&hhea)
        .map_err(|e| SwfError::Tags(format!("hhea: {e:?}")))?
        .add_table(&maxp)
        .map_err(|e| SwfError::Tags(format!("maxp: {e:?}")))?
        .add_table(&os2)
        .map_err(|e| SwfError::Tags(format!("os/2: {e:?}")))?
        .add_table(&hmtx)
        .map_err(|e| SwfError::Tags(format!("hmtx: {e:?}")))?
        .add_table(&cmap)
        .map_err(|e| SwfError::Tags(format!("cmap: {e:?}")))?
        .add_table(&loca)
        .map_err(|e| SwfError::Tags(format!("loca: {e:?}")))?
        .add_table(&glyf)
        .map_err(|e| SwfError::Tags(format!("glyf: {e:?}")))?
        .add_table(&name)
        .map_err(|e| SwfError::Tags(format!("name: {e:?}")))?
        .add_table(&post)
        .map_err(|e| SwfError::Tags(format!("post: {e:?}")))?;

    Ok(builder.build())
}
