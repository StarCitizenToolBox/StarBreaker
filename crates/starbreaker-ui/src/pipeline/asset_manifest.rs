//! Asset reference manifest collection for diagnostics and IR metadata.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AssetReferenceManifest {
    pub all_asset_refs: Vec<String>,
    pub resolved_asset_refs: Vec<String>,
    pub missing_asset_refs: Vec<String>,
}

pub(super) fn build_asset_reference_manifest(
    scene: &crate::bb_scene::BbScene,
    asset_fetcher: &dyn crate::bb_atlas::AssetFetcher,
) -> AssetReferenceManifest {
    let mut all_asset_refs = Vec::new();
    let mut resolved_asset_refs = Vec::new();
    let mut missing_asset_refs = Vec::new();
    let mut seen_assets = std::collections::BTreeSet::new();

    for node in scene.nodes.values() {
        for asset_ref in crate::ui_ir::collect_node_asset_refs(node) {
            if !seen_assets.insert(asset_ref.clone()) {
                continue;
            }
            all_asset_refs.push(asset_ref.clone());
            let resolved = asset_fetcher.fetch_image_bytes(&asset_ref).is_some()
                || asset_fetcher.fetch_svg_bytes(&asset_ref).is_some();
            if resolved {
                resolved_asset_refs.push(asset_ref);
            } else {
                missing_asset_refs.push(asset_ref);
            }
        }
    }

    AssetReferenceManifest {
        all_asset_refs,
        resolved_asset_refs,
        missing_asset_refs,
    }
}
