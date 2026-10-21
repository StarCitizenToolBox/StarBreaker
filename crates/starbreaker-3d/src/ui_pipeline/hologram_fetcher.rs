//! P4K/DataCore-backed [`HologramFetcher`] for the SELF-STATUS vehicle
//! hologram.
//!
//! The engine draws `WidgetRuntimeImage`/`Primitive` "Own Vehicle Hologram"
//! nodes as a live 3D render of the player's ship; the 2D UI compositor cannot.
//! This fetcher resolves the render scene's ROOT vehicle hull geometry from
//! DataCore (`SGeometryResourceParams`), decodes it ([`crate::parse_skin`]) and
//! rasterises a neutral greyscale shaded-filled-faces hologram (from behind, in
//! perspective) via [`starbreaker_gfx::render_vehicle_hologram`], tinted by the
//! node's authored background fill (the per-manufacturer holo colour). It is
//! generic over the loaded ship (root entity), with no ship/screen name
//! branches.

use starbreaker_datacore::loadout::EntityIndex;
use starbreaker_datacore::Database;
use starbreaker_dds::{DdsFile, ReadSibling};
use starbreaker_gfx::{
    project_radar_disc, render_vehicle_hologram, HeadingRingParams, HologramParams,
    RadarPlaneParams, RadarSpoke,
};
use starbreaker_p4k::MappedP4k;
use starbreaker_ui::pipeline::{HologramFetcher, HologramImage, RadarHeadingTape, RadarSpokeInput};

use crate::pipeline::datacore_path_to_p4k;

/// Resolves and rasterises the loaded ship's hologram on demand.
pub(super) struct P4kHologramFetcher<'a> {
    pub(super) p4k: &'a MappedP4k,
    pub(super) db: &'a Database<'a>,
    /// Render scene's root entity name (the player's vehicle). `None` for
    /// non-vehicle exports — the fetcher then yields nothing.
    pub(super) root_entity_name: Option<&'a str>,
    /// The radar plane's heading-up rotation (degrees) — the shared
    /// `FlightController/Compass/Value` (0 at static rest). Rotates the radar
    /// chrome (spokes, heading ring, cardinal markers) WITH the ship heading.
    pub(super) radar_heading_deg: f32,
}

impl P4kHologramFetcher<'_> {
    /// Resolve the root vehicle's hull geometry P4K path
    /// (`SGeometryResourceParams.Geometry.Geometry.Geometry.path`).
    fn hull_geometry_p4k_path(&self) -> Option<String> {
        let name = self.root_entity_name?;
        let idx = EntityIndex::new(self.db);
        let stem = name.rsplit('.').next().unwrap_or(name);
        let record = idx.find_record(stem)?;
        let compiled = self
            .db
            .compile_path::<String>(
                record.struct_id(),
                "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
            )
            .ok()?;
        let geom_path = self.db.query_single::<String>(&compiled, record).ok()??;
        Some(datacore_path_to_p4k(&geom_path))
    }

    /// Resolve the generic shield-face proxy mesh + its placement distance from
    /// the global `UIHoloVehicle_Config` record: `shieldProxyModel` (a curved
    /// unit pane shared by every ship's SELF-STATUS hologram) and
    /// `shieldDistance` (the fraction of the shield box the hull fills). Returns
    /// `None` if the config record or its asset is unavailable. Fully
    /// data-driven — both the asset path and the distance come from DataCore,
    /// not hard-coded values.
    fn shield_proxy(&self) -> Option<(crate::Mesh, f32)> {
        let record = self.db.records_by_type_name("UIHoloVehicle_Config").next()?;
        let model_path = self
            .db
            .compile_path::<String>(record.struct_id(), "shieldProxyModel")
            .ok()?;
        let proxy = self.db.query_single::<String>(&model_path, record).ok()??;
        let dist_path = self
            .db
            .compile_path::<f32>(record.struct_id(), "shieldDistance")
            .ok()?;
        let shield_distance = self.db.query_single::<f32>(&dist_path, record).ok()??;
        let p4k_path = datacore_path_to_p4k(&proxy);
        let companion = format!("{p4k_path}m");
        let mesh = match (self.p4k.read_file(&companion), self.p4k.read_file(&p4k_path)) {
            (Ok(verts), Ok(primary)) => crate::parse_skin_positioned(&verts, &primary).ok()?,
            (Ok(verts), Err(_)) => crate::parse_skin(&verts).ok()?,
            (Err(_), Ok(primary)) => crate::parse_skin(&primary).ok()?,
            (Err(_), Err(_)) => return None,
        };
        Some((mesh, shield_distance))
    }
}

impl HologramFetcher for P4kHologramFetcher<'_> {
    fn fetch_vehicle_hologram(&self, width: u32, height: u32, tint: [f32; 4]) -> Option<HologramImage> {
        if width == 0 || height == 0 {
            return None;
        }
        let p4k_path = self.hull_geometry_p4k_path()?;
        // Vertex data lives in the `m` companion (`.cgam`/`.cgfm`/`.skinm`);
        // the bare primary file holds the scene-graph/NMC. The full (non-LOD)
        // hull is used deliberately: the LOD variants don't decode cleanly yet.
        // `parse_skin_positioned` bakes the NMC node transforms so multi-part
        // geometry (wings, both engine pods, sub-objects) is assembled in world
        // space — raw `parse_skin` leaves parts detached / at the origin.
        let companion = format!("{p4k_path}m");
        let mut mesh = match (self.p4k.read_file(&companion), self.p4k.read_file(&p4k_path)) {
            (Ok(verts), Ok(primary)) => crate::parse_skin_positioned(&verts, &primary).ok()?,
            (Ok(verts), Err(_)) => crate::parse_skin(&verts).ok()?,
            (Err(_), Ok(primary)) => crate::parse_skin(&primary).ok()?,
            (Err(_), Err(_)) => return None,
        };
        if mesh.positions.is_empty() || mesh.indices.len() < 3 {
            return None;
        }
        // Wrap the hull in 4 shield faces forming a box PROPORTIONED to the
        // hull, placed via the data-driven `shieldDistance` (the hull fills that
        // fraction of the box → natural corner gaps). The proxy mesh + distance
        // both come from `UIHoloVehicle_Config`; only the fainter alpha is a
        // SELF-STATUS framing choice. `shield_index_start` marks where the
        // shield triangles begin so the renderer can fade them vs the hull.
        // Shields read as a faint see-through cage, well below the hull alpha.
        const SELF_STATUS_SHIELD_ALPHA_SCALE: f32 = 0.30;
        // Owner-tuned view choice (like the framing constants below): shrink the
        // shield box slightly toward the hull origin so the faces read smaller
        // and hug the hull more closely — keeps them clear of the left/right
        // image edges at this fill. The box GEOMETRY stays data-driven
        // (`with_shield_panes` / `shieldDistance`); this only scales the result.
        const SELF_STATUS_SHIELD_BOX_SCALE: f32 = 0.82;
        let hull_vtx = mesh.positions.len();
        let shield_index_start = match self.shield_proxy() {
            Some((pane, shield_distance)) if !pane.positions.is_empty() => {
                let (mut merged, boundary) = crate::with_shield_panes(mesh, &pane, shield_distance);
                for p in merged.positions.iter_mut().skip(hull_vtx) {
                    p[0] *= SELF_STATUS_SHIELD_BOX_SCALE;
                    p[1] *= SELF_STATUS_SHIELD_BOX_SCALE;
                    p[2] *= SELF_STATUS_SHIELD_BOX_SCALE;
                }
                mesh = merged;
                Some(boundary)
            }
            _ => None,
        };
        // SELF-STATUS framing, calibrated to the in-game reference: yaw 0°
        // (nose pointing AWAY / up the frame), pitched -30° so the nose dips
        // DOWN, with perspective, filling roughly half the diagram area.
        // Semi-transparent faces (low alpha) so overlapping panels read as a
        // see-through hologram. The engine camera FOV/distance for the runtime
        // primitive isn't decoded, so these are a view choice matched to the
        // reference, not a layout fudge.
        const SELF_STATUS_YAW_DEG: f32 = 0.0;
        const SELF_STATUS_TILT_BACK_DEG: f32 = -30.0;
        // The HULL fills ~65% of its longest image axis (the rasteriser fits the
        // hull, not the shields, and centres on the origin); the shield box then
        // frames the hull inside the image edges.
        const SELF_STATUS_FIT: f32 = 0.75;
        // Filled, shaded faces only (no wireframe) with a strong perspective so
        // the flat hull reads as an angled 3D hologram rather than a top-down
        // silhouette.
        const SELF_STATUS_FACE_ALPHA: f32 = 0.25;
        const SELF_STATUS_WIRE_ALPHA: f32 = 0.0;
        const SELF_STATUS_PERSPECTIVE: f32 = 1.5;
        let params = HologramParams {
            tint,
            yaw_deg: SELF_STATUS_YAW_DEG,
            tilt_back_deg: SELF_STATUS_TILT_BACK_DEG,
            fit: SELF_STATUS_FIT,
            face_alpha: SELF_STATUS_FACE_ALPHA,
            wire_alpha: SELF_STATUS_WIRE_ALPHA,
            perspective: SELF_STATUS_PERSPECTIVE,
            shield_index_start,
            shield_alpha_scale: SELF_STATUS_SHIELD_ALPHA_SCALE,
        };
        let img = render_vehicle_hologram(&mesh.positions, &mesh.indices, width, height, &params);
        Some(HologramImage {
            width,
            height,
            rgba: img.into_raw(),
        })
    }

    fn fetch_radar_plane(
        &self,
        width: u32,
        height: u32,
        tint: [f32; 3],
        disc_material_path: &str,
        sweep_material_path: Option<&str>,
        spoke_material_path: Option<&str>,
        spokes: &[RadarSpokeInput],
        heading: Option<RadarHeadingTape<'_>>,
    ) -> Option<HologramImage> {
        if width == 0 || height == 0 {
            return None;
        }
        // Resolve the disc material → its diffuse texture: the REAL radar-disc
        // art (concentric rings + degree-tick scale + axis). `disc_material_path`
        // is already the brand-correct material (the IR `primitive_material`
        // reflects the per-manufacturer override — DRAK keeps the generic,
        // Greycat/RSI swap to `ui_grin_…`/`…_RSI`). Decode the texture and
        // project it as the RTT window camera would (tilted disc → ellipse),
        // tinted by `tint` (the brand accent). The art + tint are DATA; only the
        // camera tilt is owner-tuned (the engine runtime camera transform is
        // absent at static rest — the hologram-camera boundary).
        let mtl_bytes = super::p4k_fetchers::read_p4k_asset(self.p4k, disc_material_path)?;
        let mtl = crate::mtl::parse_mtl(&mtl_bytes).ok()?;
        let sub = mtl.materials.first()?;
        let tex_ref = mtl.materials.iter().find_map(|sm| sm.diffuse_tex.clone())?;
        let texture = decode_p4k_dds_rgba(self.p4k, &tex_ref)?;
        // The disc material's `ViewingAngle` PublicParam (the radial shader's
        // view angle) orients the texture: the DRAK radar authors 180°, so the
        // disc reads rotated vs the raw texture. Data-backed + per-manufacturer.
        let texture_rotation_deg = sub.public_param_f32(&["ViewingAngle"]).unwrap_or(0.0);

        // The rotating sweep wedge (`r_radarmapscreen_idle_animation`), loaded from
        // its brand-resolved material. The sweep is a live animation; at static
        // rest it's drawn once at an owner-tuned rest-frame angle.
        let sweep_texture = sweep_material_path
            .and_then(|path| super::p4k_fetchers::read_p4k_asset(self.p4k, path))
            .and_then(|bytes| crate::mtl::parse_mtl(&bytes).ok())
            .and_then(|m| m.materials.iter().find_map(|sm| sm.diffuse_tex.clone()))
            .and_then(|tex| decode_p4k_dds_rgba(self.p4k, &tex));
        // The wedge apex (pivot → disc centre) + radial extent are COMPUTED from
        // the decoded sweep texture, so a per-manufacturer sweep texture tracks
        // automatically (no hard-coded apex).
        let (sweep_apex, sweep_tex_radius) = sweep_texture
            .as_ref()
            .and_then(starbreaker_gfx::sweep_wedge_geometry)
            .unwrap_or(([0.5, 0.5], 0.5));

        // The authored spokes (`Circle_Line_*`): their geometry + per-spoke colour
        // arrive from the IR (`spokes`); the soft-glow APPEARANCE is read from the
        // brand-resolved `line_a` material — `Glow` (the bloom amount), and the
        // `OuterAlpha`/`InnerAlpha` PublicParams (the bar's `Gradient` fade from
        // rim to centre). All data-backed + per-manufacturer (a brand-swapped
        // material or a `.mtl` edit flows straight through). Defaults match the
        // shader's own defaults when a param is absent.
        let spoke_mtl = spoke_material_path
            .and_then(|path| super::p4k_fetchers::read_p4k_asset(self.p4k, path))
            .and_then(|bytes| crate::mtl::parse_mtl(&bytes).ok());
        let spoke_sub = spoke_mtl.as_ref().and_then(|m| m.materials.first());
        let spoke_glow = spoke_sub.map(|s| s.glow).unwrap_or(0.0);
        let spoke_outer_alpha = spoke_sub
            .and_then(|s| s.public_param_f32(&["OuterAlpha"]))
            .unwrap_or(1.0);
        let spoke_inner_alpha = spoke_sub
            .and_then(|s| s.public_param_f32(&["InnerAlpha"]))
            .unwrap_or(1.0);
        let radar_spokes: Vec<RadarSpoke> = spokes
            .iter()
            .map(|s| RadarSpoke {
                anchor: s.anchor,
                length_frac: s.length_frac,
                width_frac: s.width_frac,
                rotation_deg: s.rotation_deg,
                colour: s.colour,
                alpha: s.alpha,
            })
            .collect();

        // The emissive/bloom brightness — the ONE render value (the radar screen is
        // an emissive/bloomed surface; the absolute brightness is a render-pipeline
        // effect not in the static data, shared by the disc + spokes + ring).
        const RADAR_EMISSIVE: f32 = 3.0;
        // The outer heading-tape ring (`HeadingTape`): load the brand-resolved tape
        // material's atlas (`coordinates_novalue` tick row) + carry the node's
        // authored UV window. Atlas + UV + fill alpha are DATA; the ring alpha is
        // the tape's authored fill (`fill_alpha`, the brand `Base` ~0.3) × the one
        // emissive boost — not a picked constant.
        let heading_texture = heading
            .map(|h| h.material_path)
            .and_then(|path| super::p4k_fetchers::read_p4k_asset(self.p4k, path))
            .and_then(|bytes| crate::mtl::parse_mtl(&bytes).ok())
            .and_then(|m| m.materials.iter().find_map(|sm| sm.diffuse_tex.clone()))
            .and_then(|tex| decode_p4k_dds_rgba(self.p4k, &tex));
        // The cardinal "exit" markers (`⌐▽⌐`) are the 9th column of the atlas's
        // coordinate-label grid (the `coordinates_novalue` atlas has 8 degree
        // labels + 1 cardinal glyph per row → 9 columns; confirmed by the degree
        // readout's `1/9` cell width). The 9th column, top row, is the exit glyph;
        // its row height is the tape's authored `UVSize.y`. Drawn at N/E/S/W,
        // heading-rotated. Tinted by the brand accent (the `NorthPoint` cardinal
        // colour family).
        // The `coordinates_novalue` atlas is a degree grid of 4 rows × 9 columns =
        // 36 cells (8 labels + 1 cardinal glyph per row); the 9th column is the
        // cardinal `└▽┘`. 36 cells = the full 360° heading scale, so each degree
        // cell is 1/36 of the ring (matching `HeadingDegreeReadout` W = 0.02778).
        const DEGREE_GRID_ROWS: f32 = 4.0;
        const DEGREE_GRID_COLUMNS: f32 = 9.0;
        let cardinal_cell_fraction = 1.0 / (DEGREE_GRID_ROWS * DEGREE_GRID_COLUMNS);
        // The cardinal glyph cell is COMPUTED from the atlas (the `└▽┘` bbox in the
        // last column's top row) so it captures the full glyph (no clip) and tracks
        // a per-manufacturer atlas; fall back to the nominal 9th-column cell.
        let (cardinal_uv_start, cardinal_uv_size) = heading_texture
            .as_ref()
            .and_then(|atlas| starbreaker_gfx::cardinal_marker_cell(atlas, DEGREE_GRID_COLUMNS))
            .unwrap_or((
                [(DEGREE_GRID_COLUMNS - 1.0) / DEGREE_GRID_COLUMNS, 0.0],
                [1.0 / DEGREE_GRID_COLUMNS, 0.063],
            ));
        let heading_ring = heading.map(|h| HeadingRingParams {
            uv_start: h.uv_start,
            uv_size: h.uv_size,
            alpha: h.fill_alpha * RADAR_EMISSIVE,
            cardinal_uv_start,
            cardinal_uv_size,
            cardinal_colour: tint,
            cardinal_cell_fraction,
        });

        // ~37° matches the reference ellipse (minor/major ≈ 0.6). Owner-tuned.
        const RADAR_TILT_DEG: f32 = 37.0;
        // Rest-frame sweep position (the beam rotates live; this is the captured
        // frame — owner-tuned, like the tilt).
        const RADAR_SWEEP_ANGLE_DEG: f32 = 45.0;
        const RADAR_SWEEP_ALPHA: f32 = 0.5;
        let params = RadarPlaneParams {
            tilt_deg: RADAR_TILT_DEG,
            tint_rgb: tint,
            intensity: RADAR_EMISSIVE,
            texture_rotation_deg,
            sweep_alpha: if sweep_texture.is_some() { RADAR_SWEEP_ALPHA } else { 0.0 },
            sweep_angle_deg: RADAR_SWEEP_ANGLE_DEG,
            sweep_apex,
            sweep_tex_radius,
            heading_deg: self.radar_heading_deg,
            // The crisp `Circle_Ripple` boundary stroke is NOT in the reference —
            // the disc texture provides the soft rim. Leave it off.
            outer_ring_alpha: 0.0,
            spokes: radar_spokes,
            spoke_outer_alpha,
            spoke_inner_alpha,
            spoke_glow,
            heading_ring,
            ..Default::default()
        };
        let disc = project_radar_disc(
            width,
            height,
            &texture,
            sweep_texture.as_ref(),
            heading_texture.as_ref(),
            &params,
        );
        Some(HologramImage {
            width,
            height,
            rgba: disc.into_raw(),
        })
    }
}

/// Reads split-mip DDS sibling files (`.1`, `.2`, … / `.dds.a`) from the P4K for
/// [`DdsFile::from_split`].
struct RadarDdsSiblingReader<'a> {
    p4k: &'a MappedP4k,
    base_path: String,
}

impl ReadSibling for RadarDdsSiblingReader<'_> {
    fn read_sibling(&self, suffix: &str) -> Option<Vec<u8>> {
        let path = format!("{}{suffix}", self.base_path);
        self.p4k
            .entry_case_insensitive(&path)
            .and_then(|entry| self.p4k.read(entry).ok())
    }
}

/// Decode a P4K DDS texture (handling split-mip siblings) to RGBA, resolving the
/// texture reference's actual archive path via the shared asset candidates
/// (`.tif` → `.dds`, `Data\` prefix).
fn decode_p4k_dds_rgba(p4k: &MappedP4k, tex_ref: &str) -> Option<image::RgbaImage> {
    let base_path = super::p4k_fetchers::p4k_asset_candidates(tex_ref)
        .into_iter()
        .find(|candidate| p4k.entry_case_insensitive(candidate).is_some())?;
    let data = p4k.read_file(&base_path).ok()?;
    let reader = RadarDdsSiblingReader { p4k, base_path };
    let dds = DdsFile::from_split(&data, &reader)
        .or_else(|_| DdsFile::headers_only(&data))
        .ok()?;
    let rgba = dds.decode_rgba(0).ok()?;
    let (w, h) = dds.dimensions(0);
    image::RgbaImage::from_raw(w, h, rgba)
}
