//! Socpak path -> (category, subcategory) classification for the export view.
//!
//! Every `.socpak` lives under `Data\ObjectContainers\`, and CIG's folder
//! layout beneath that root is the only complete, reliable taxonomy of object
//! container types: the inner `<ObjectContainer>` `type`/`tags` attributes are
//! empty or a generic `"Default OC"`, and the `StaticEntityTags` GUIDs resolve
//! to per-entity gameplay/AI tags (e.g. `LedgeObject`) shared across every
//! container kind rather than a container category. This mirrors the entity
//! exporter, which already derives ship/vehicle/weapon buckets from a record's
//! file path (`output_object_type_directory_for_record`).
//!
//! `categorize_socpak` maps a path to a stable two-tier category/subcategory
//! purely from its path segments — only structural prefixes, never a specific
//! asset name — so the UI can present collapsible groups over the ~9.5k
//! containers.

/// A two-tier classification of a socpak, derived from its P4k path.
pub struct SocpakCategory {
    /// Stable top-level group (e.g. `"Space Stations"`). Always one of a fixed
    /// set so the frontend can order groups deterministically.
    pub category: &'static str,
    /// Human-readable second tier within the category (e.g. a manufacturer, a
    /// city, or a building-set kind). Title-cased from the path.
    pub subcategory: String,
}

impl SocpakCategory {
    fn new(category: &'static str, subcategory: impl Into<String>) -> Self {
        Self { category, subcategory: subcategory.into() }
    }
}

/// Classify a socpak P4k path into a display category + subcategory.
///
/// The match is relative to the `ObjectContainers/` root, so a `Data\` prefix
/// (or its absence) does not affect the result. Unknown layouts fall back to
/// `"Locations — Other"` / `"Other"` rather than panicking.
pub fn categorize_socpak(path: &str) -> SocpakCategory {
    let norm = path.replace('\\', "/");
    let lower_full = norm.to_ascii_lowercase();
    // Classify relative to the ObjectContainers root so the Data\ prefix is
    // irrelevant. ASCII lowercasing preserves byte length, so the same offset
    // indexes both the lower and original strings.
    let rel_start = lower_full
        .find("objectcontainers/")
        .map(|i| i + "objectcontainers/".len())
        .unwrap_or(0);
    let lower = &lower_full[rel_start..];
    let orig = &norm[rel_start..];

    let seg: Vec<&str> = lower.split('/').filter(|s| !s.is_empty()).collect();
    let seg_orig: Vec<&str> = orig.split('/').filter(|s| !s.is_empty()).collect();
    let at = |i: usize| seg.get(i).copied().unwrap_or("");
    let disp = |i: usize| seg_orig.get(i).map(|s| titlecase(s));

    match at(0) {
        "ships" => SocpakCategory::new("Ships", disp(1).unwrap_or_else(|| "Other".into())),
        "vehicles" => {
            SocpakCategory::new("Ground Vehicles", disp(1).unwrap_or_else(|| "Other".into()))
        }
        "setup" => {
            SocpakCategory::new("Gameplay Setup", disp(1).unwrap_or_else(|| "General".into()))
        }
        "ac" => SocpakCategory::new("Game Modes & Test Maps", "Arena Commander"),
        "sm" => SocpakCategory::new("Game Modes & Test Maps", "Star Marine"),
        "ea" => SocpakCategory::new(
            "Game Modes & Test Maps",
            disp(1).unwrap_or_else(|| "Event/Arena".into()),
        ),
        "demo" => SocpakCategory::new("Game Modes & Test Maps", "Demo"),
        "frontend" => SocpakCategory::new("Game Modes & Test Maps", "Frontend"),
        "test" => SocpakCategory::new("Game Modes & Test Maps", "Test"),
        "pu" => categorize_pu(&seg, &disp),
        _ => SocpakCategory::new("Other", "Uncategorised"),
    }
}

/// Classify the `PU/<bucket>/...` containers (the bulk of the corpus).
fn categorize_pu(seg: &[&str], disp: &dyn Fn(usize) -> Option<String>) -> SocpakCategory {
    let at = |i: usize| seg.get(i).copied().unwrap_or("");
    // Hangar geometry is scattered across building sets — a handful of
    // standalone `PU/Hangars/...` plus the bulk under `loc/mod/common/hangar`
    // and `loc/mod/station/.../hangar`. Group every PU container whose path
    // carries a `hangar` segment so the size/variant pieces are findable in one
    // place rather than buried in Shared Modules / Space Stations.
    if seg.iter().any(|s| s.contains("hangar")) {
        return SocpakCategory::new("Hangars", hangar_subcategory(seg, disp));
    }
    match at(1) {
        "loc" => categorize_loc(seg, disp),
        "system" => {
            SocpakCategory::new("Planet & System Set-Dressing", disp(2).unwrap_or_else(|| "System".into()))
        }
        "derelict" => {
            SocpakCategory::new("Derelicts & Wrecks", disp(2).unwrap_or_else(|| "Derelict".into()))
        }
        "wreckage" => SocpakCategory::new("Derelicts & Wrecks", "Wreckage"),
        "hijackedships" => SocpakCategory::new("Derelicts & Wrecks", "Hijacked Ships"),
        "shops" => {
            SocpakCategory::new("Shops & Interiors", disp(2).unwrap_or_else(|| "General".into()))
        }
        "surfaceop" => SocpakCategory::new(
            "Outposts & Surface Bases",
            disp(2).unwrap_or_else(|| "Surface Outpost".into()),
        ),
        "station" => {
            SocpakCategory::new("Space Stations", disp(2).unwrap_or_else(|| "Station".into()))
        }
        "cityset_buildings" => SocpakCategory::new("Cities & Landing Zones", "City Buildings"),
        "asteroid" | "asteroidcluster" => {
            SocpakCategory::new("Planet & System Set-Dressing", "Asteroids")
        }
        "jumppoint" => SocpakCategory::new("Planet & System Set-Dressing", "Jump Points"),
        "props" | "flair" | "design" | "modular" | "junk" | "human" => {
            SocpakCategory::new("Props, Flair & Decor", disp(1).unwrap_or_else(|| "Props".into()))
        }
        "racing" | "rctrk" => SocpakCategory::new("Game Modes & Test Maps", "Racing"),
        "shipcombatoverlay" => {
            SocpakCategory::new("Game Modes & Test Maps", "Ship Combat Overlay")
        }
        "missions" => SocpakCategory::new("Game Modes & Test Maps", "Missions"),
        _ => SocpakCategory::new("Locations — Other", disp(1).unwrap_or_else(|| "Other".into())),
    }
}

/// Classify `PU/loc/...` — modular building sets, flagship cities, surface.
fn categorize_loc(seg: &[&str], disp: &dyn Fn(usize) -> Option<String>) -> SocpakCategory {
    let at = |i: usize| seg.get(i).copied().unwrap_or("");
    match at(2) {
        // PU/loc/flagship/<system>/<city>/... — the named landing zones.
        "flagship" => {
            let city = disp(4).or_else(|| disp(3)).unwrap_or_else(|| "Landing Zone".into());
            SocpakCategory::new("Cities & Landing Zones", city)
        }
        "surface" => SocpakCategory::new("Outposts & Surface Bases", "Surface"),
        "space" => SocpakCategory::new("Outposts & Surface Bases", "Space"),
        "mod" => categorize_loc_mod(seg, disp),
        _ => SocpakCategory::new("Locations — Other", disp(2).unwrap_or_else(|| "Other".into())),
    }
}

/// Classify `PU/loc/mod/<set>/...` — the modular set-dressing building blocks
/// that make up the majority of all containers.
fn categorize_loc_mod(seg: &[&str], disp: &dyn Fn(usize) -> Option<String>) -> SocpakCategory {
    let at = |i: usize| seg.get(i).copied().unwrap_or("");
    match at(3) {
        "station" => SocpakCategory::new("Space Stations", "Modular Station Modules"),
        "outpost" => SocpakCategory::new("Outposts & Surface Bases", "Modular Outposts"),
        "fob" => SocpakCategory::new("Outposts & Surface Bases", "Forward Operating Bases"),
        "drlct" => SocpakCategory::new("Derelicts & Wrecks", "Modular Derelicts"),
        "ugf" | "ug_facility" => SocpakCategory::new("Underground & Caves", "Underground Facilities"),
        "cave" => SocpakCategory::new("Underground & Caves", "Caves"),
        "sewers" => SocpakCategory::new("Underground & Caves", "Sewers"),
        "prison" => SocpakCategory::new("Underground & Caves", "Prisons"),
        "pyro" | "stanton" | "nyx" => SocpakCategory::new(
            "Planet & System Set-Dressing",
            disp(3).unwrap_or_else(|| "System".into()),
        ),
        "common" => SocpakCategory::new("Shared Modules & Lighting", "Common Modules"),
        "lighting" => SocpakCategory::new("Shared Modules & Lighting", "Lighting"),
        _ => SocpakCategory::new("Locations — Other", disp(3).unwrap_or_else(|| "Modular".into())),
    }
}

/// Label a hangar container by its architecture family / variant folder.
///
/// `PU/Hangars/<family>` uses `<family>` (selfland, aeroview…). Otherwise the
/// family is the folder around the `hangar` segment — the directory just after
/// it (`common/hangar/util_a` -> `Util A`), or the one before it as a fallback.
fn hangar_subcategory(seg: &[&str], disp: &dyn Fn(usize) -> Option<String>) -> String {
    if seg.get(1) == Some(&"hangars") {
        return disp(2).unwrap_or_else(|| "Hangar".into());
    }
    if let Some(i) = seg.iter().position(|s| s.contains("hangar")) {
        if let Some(next) = seg.get(i + 1) {
            if !next.ends_with(".socpak") {
                return disp(i + 1).unwrap_or_else(|| "Hangar".into());
            }
        }
        if i > 0 {
            return disp(i - 1).unwrap_or_else(|| "Hangar".into());
        }
    }
    "Hangar".into()
}

/// Title-case a path segment for display, preserving all-caps codes.
///
/// Manufacturer-style tokens that are already entirely upper-case (`AEGS`,
/// `RSI`, `DRAK`) are kept verbatim; everything else is split on `_` and each
/// word's first letter upper-cased (`ez_hab` -> `Ez Hab`, `orison` -> `Orison`).
fn titlecase(s: &str) -> String {
    if s.len() > 1 && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return s.to_string();
    }
    s.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::categorize_socpak;

    fn cat(path: &str) -> (&'static str, String) {
        let c = categorize_socpak(path);
        (c.category, c.subcategory)
    }

    #[test]
    fn data_prefix_is_optional() {
        assert_eq!(cat("Data\\ObjectContainers\\PU\\Hangars\\selfland\\h.socpak").0, "Hangars");
        assert_eq!(cat("PU/Hangars/selfland/h.socpak").0, "Hangars");
    }

    #[test]
    fn cities_use_the_city_name_as_subcategory() {
        assert_eq!(
            cat("Data/ObjectContainers/PU/loc/flagship/stanton/orison/orison_hotel.socpak"),
            ("Cities & Landing Zones", "Orison".to_string())
        );
        assert_eq!(
            cat("Data/ObjectContainers/PU/loc/flagship/stanton/lorville/lv.socpak"),
            ("Cities & Landing Zones", "Lorville".to_string())
        );
        assert_eq!(
            cat("Data/ObjectContainers/PU/cityset_buildings/b.socpak"),
            ("Cities & Landing Zones", "City Buildings".to_string())
        );
    }

    #[test]
    fn stations_cover_modular_and_named() {
        assert_eq!(
            cat("Data/ObjectContainers/PU/loc/mod/station/core/s.socpak"),
            ("Space Stations", "Modular Station Modules".to_string())
        );
        assert_eq!(
            cat("Data/ObjectContainers/PU/station/motel/grimhex.socpak"),
            ("Space Stations", "Motel".to_string())
        );
    }

    #[test]
    fn outposts_and_surface_bases() {
        assert_eq!(cat("PU/loc/mod/outpost/o.socpak").0, "Outposts & Surface Bases");
        assert_eq!(cat("PU/loc/mod/fob/o.socpak").1, "Forward Operating Bases".to_string());
        assert_eq!(cat("PU/surfaceop/landing_pad/p.socpak").1, "Landing Pad".to_string());
        assert_eq!(cat("PU/loc/surface/x.socpak"), ("Outposts & Surface Bases", "Surface".into()));
    }

    #[test]
    fn underground_and_caves() {
        assert_eq!(cat("PU/loc/mod/ugf/asd/u.socpak").1, "Underground Facilities".to_string());
        assert_eq!(cat("PU/loc/mod/cave/rock01/c.socpak").1, "Caves".to_string());
        assert_eq!(cat("PU/loc/mod/prison/p.socpak"), ("Underground & Caves", "Prisons".into()));
    }

    #[test]
    fn derelicts_and_wrecks() {
        assert_eq!(cat("PU/loc/mod/drlct/d.socpak").1, "Modular Derelicts".to_string());
        assert_eq!(
            cat("PU/derelict/DRAK/caterpillar/c.socpak"),
            ("Derelicts & Wrecks", "DRAK".to_string())
        );
        assert_eq!(cat("PU/HijackedShips/h.socpak").1, "Hijacked Ships".to_string());
    }

    #[test]
    fn planet_and_system_set_dressing() {
        assert_eq!(
            cat("PU/system/stanton/stanton.socpak"),
            ("Planet & System Set-Dressing", "Stanton".to_string())
        );
        assert_eq!(cat("PU/loc/mod/pyro/p.socpak").0, "Planet & System Set-Dressing");
        assert_eq!(cat("PU/asteroidCluster/a.socpak").1, "Asteroids".to_string());
    }

    #[test]
    fn ships_and_vehicles_use_manufacturer() {
        assert_eq!(
            cat("Data/ObjectContainers/Ships/AEGS/Gladius/body.socpak"),
            ("Ships", "AEGS".to_string())
        );
        assert_eq!(
            cat("Data/ObjectContainers/Vehicles/GRIN/MTC/rear.socpak"),
            ("Ground Vehicles", "GRIN".to_string())
        );
    }

    #[test]
    fn setup_and_game_modes() {
        assert_eq!(cat("Setup/elevator_setup/e.socpak"), ("Gameplay Setup", "Elevator Setup".into()));
        assert_eq!(cat("AC/DyingStar/global.socpak").0, "Game Modes & Test Maps");
        assert_eq!(cat("SM/FPS_Demien/global.socpak"), ("Game Modes & Test Maps", "Star Marine".into()));
        assert_eq!(cat("EA/tow/Phase2/outpost.socpak"), ("Game Modes & Test Maps", "Tow".into()));
    }

    #[test]
    fn shops_props_and_shared_modules() {
        assert_eq!(cat("PU/Shops/admin/admin_grimhex.socpak"), ("Shops & Interiors", "Admin".into()));
        assert_eq!(cat("PU/props/p.socpak"), ("Props, Flair & Decor", "Props".into()));
        assert_eq!(cat("PU/loc/mod/common/c.socpak"), ("Shared Modules & Lighting", "Common Modules".into()));
    }

    #[test]
    fn hangars_gather_scattered_modules() {
        // Standalone and the bulk modular hangar sets all land under Hangars.
        assert_eq!(cat("PU/Hangars/selfland/h.socpak"), ("Hangars", "Selfland".into()));
        assert_eq!(
            cat("PU/loc/mod/common/hangar/util_a/hangar_lrgtop_001.socpak"),
            ("Hangars", "Util A".into())
        );
        assert_eq!(cat("PU/loc/mod/station/util_a/int/module/hangar/x/y.socpak").0, "Hangars");
        // Ship and gameplay-setup hangars keep their primary top-level type.
        assert_eq!(cat("Ships/RSI/Polaris/rsi_polaris_int_hangar.socpak").0, "Ships");
        assert_eq!(cat("Setup/elevator_setup/playerhangar/e.socpak").0, "Gameplay Setup");
    }

    #[test]
    fn loose_root_socpaks_are_uncategorised() {
        assert_eq!(cat("Data/ObjectContainers/levski_topdeck.socpak"), ("Other", "Uncategorised".into()));
    }
}
