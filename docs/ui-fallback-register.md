# UI Fallback Register

This register tracks active UI fallbacks in `starbreaker-ui` and explicitly records owner, scope, trigger signal, and retirement target.

## Active Fallbacks

| Component | Fallback | Owner | Scope | Trigger Signal | Retirement Target |
| --- | --- | --- | --- | --- | --- |
| `crates/starbreaker-ui/src/bb_atlas.rs` | Manufacturer/gen atlas fallback paths | UI pipeline | Asset resolution when exact texture path is missing | `fallback_counter` in pipeline diagnostics and unresolved asset refs | Replace with authoritative source path resolution for all certified families in Phase 6B |
| `crates/starbreaker-ui/src/bb_loc.rs` | Ordinal localization parameter fallback | UI data resolution | Missing explicit named localization parameters | Localization fallback counters + unresolved localization warnings | Retire when localization map coverage proves complete on certification sets |
| `crates/starbreaker-ui/src/defaults.rs` | Versioned static default-value registry | UI data policy | Placeholder values for unresolved bindings/localization | `fallback_counter` diagnostics + unresolved binding warnings | Replace with authoritative runtime/default source extraction after fallback trigger rate is near-zero |
| `crates/starbreaker-ui/src/style.rs` | Drake amber style fallback | UI style selection | Missing/invalid style source resolution | Selected style provenance + fallback warnings | Retire once style selection is always sourced from canonical style data for certified families |
| `crates/starbreaker-ui/src/bb_layout.rs` | Unknown sizing behavior defaults to fill-parent | Layout resolver | Unknown `sizingBehavior` values | Layout warning telemetry and unusual rect diffs in certification output | Replace with explicit sizing behavior coverage + hard-fail in non-release checks |

## Recently Retired / Reduced Fallbacks

| Component | Previous Fallback | Status | Evidence |
| --- | --- | --- | --- |
| `crates/starbreaker-ui/src/pipeline.rs` | Pipeline-local defaults builder and split fallback policy | Retired | Consolidated to `DefaultValueRegistry::with_pipeline_defaults(...)` in Phase 4 |
| `crates/starbreaker-ui/src/ir_compose.rs` | Name/path-based medical/manufacturer hardcoded rendering branches | Retired | Eliminated in Phase 2B source-backed IR pass and guarded by `.github/scripts/check_ui_hardcoding.sh` |

## Operational Policy

- Every fallback in production must be listed here.
- New fallbacks must declare scope, trigger, and sunset target before merge.
- Fall-backs without telemetry or retirement criteria are not allowed.
- When a fallback is removed, move it to the retired section with evidence.
