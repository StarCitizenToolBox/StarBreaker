# StarBreaker Copilot instructions

StarBreaker is a Rust workspace for inspecting and exporting Star Citizen data
from `Data.p4k`. The workspace includes the CLI (`cli/`, binary
`starbreaker`), reusable format crates (`crates/starbreaker-*`), an MCP server
(`mcp/`), a Tauri/React app (`app/`), an Astro docs site (`website/`), and a
Blender addon (`blender_addon/`). Read `AGENTS.md` first for the full project
rules; read `blender_addon/AGENTS.md` before editing addon code.

The parent workspace also has root-level instructions at
`../.github/copilot-instructions.md`. Read and follow those instructions too,
especially the mandatory Visual Fix Loop (VFL) for Blender-visible behavior,
addon/Tauri UI behavior, subjective visual fixes, and user-verified performance
work. If this repo is opened directly and the parent file is not visible, apply
the VFL rules from the workspace copy before committing visual or interactive
fixes.

## Build, test, and lint commands

Run commands from the StarBreaker git root, not the parent `scorg_tools/`
workspace.

```bash
# Rust iteration build; dev profile has opt-level=1 and optimized deps.
cargo build

# Release CLI build, used for final exporter runs and CI artifacts.
cargo build --release -p starbreaker

# Full Rust workspace tests.
cargo test --workspace

# CI-equivalent Rust checks.
cargo build --release --workspace
cargo test --release --workspace

# Target one crate or one test by substring.
cargo test -p starbreaker-3d --lib
cargo test -p starbreaker-3d validate_lights_extraction -- --nocapture

# Lint/format Rust when the components are installed.
cargo clippy --workspace --all-targets
cargo fmt --all
```

```bash
# Blender addon tests use stubbed bpy and run on system Python.
cd blender_addon
python3 -m unittest discover -s tests -q
python3 -m unittest discover -s tests -p 'test_manifest.py' -q
```

```bash
# Tauri app frontend.
cd app
npm ci
npm run build
npm run lint
npm run tauri build

# Documentation website.
cd website
npm ci
npm run build
```

For native decomposed `.blend` export performance/regression work, use the
checked-in profiler:

```bash
tools/profile_aurora_blend.sh /tmp/starbreaker_aurora_profile 0
```

The CLI auto-detects common Star Citizen install paths. Set `SC_DATA_P4K` only
when targeting a non-default install. The canonical Aurora decomposed export is:

```bash
SC_DATA_P4K="$HOME/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k" \
  target/release/starbreaker entity export "aurora_mk2" \
  "../ships" --kind decomposed --format blend --lod 0 --mip 0 --materials all
```

## Architecture

- `crates/starbreaker-3d` owns entity export. The pipeline is split under
  `src/pipeline/`: `loadout.rs` resolves DataCore loadouts and item ports,
  `interiors.rs` handles interior placements, `textures.rs` and `palette.rs`
  build shared package assets, `glb_assembly.rs` assembles GLB output, and
  `blend_assembly.rs` writes native `.blend` package scenes and decomposed
  asset libraries. `decomposed.rs` coordinates package output and shared
  `Data/...` asset reuse.
- Lower-level format crates are deliberately separate: `starbreaker-p4k` reads
  archives, `starbreaker-datacore` parses the database, `starbreaker-cryxml`
  decodes binary XML, `starbreaker-chunks` handles CryEngine chunk files,
  `starbreaker-dds` decodes textures, and Wwise/WEM/CHF crates cover audio and
  character formats.
- The decomposed export contract is shared between Rust and Blender. Rust emits
  `Packages/<package>/scene.json`, `palettes.json`, `liveries.json`, material
  sidecars, textures, and reusable assets under `Data/...`; the Blender addon
  imports those packages and reconstructs materials, lights, palette/livery
  state, and animation controls.
- `blender_addon/starbreaker_addon/runtime/importer/` composes
  `PackageImporter` from mixins: palette, decals, layers, materials, builders,
  groups, and orchestration. Keep new per-entity import behavior in focused
  helpers or mixins rather than growing orchestration.
- The Tauri app wraps the Rust crates from `app/src-tauri` and uses the React
  frontend in `app/src`. The website is independent Astro/Starlight content in
  `website/`.

## MCP workflows

- StarBreaker MCP is the preferred way to inspect game data while researching
  exporter bugs. Use it for fast DataCore, P4k, material, texture, chunk, and
  animation lookups instead of writing one-off CLI probes.
- Use `search_entities` for EntityClassDefinition lookup, `search_records` for
  all DataCore record types, `datacore_record` for full JSON records, and
  `datacore_query` for a specific property path. For raw loadout data, query
  `Components[SEntityComponentDefaultLoadoutParams]`; `entity_loadout` returns
  StarBreaker's processed/resolved tree.
- Use `p4k_search`, `p4k_list`, and `p4k_read` to find and inspect archive
  paths. Use `mtl_summary` for material XML summaries, `image_preview` for
  DDS/PNG/JPG visual checks, `chunk_list`/`chunk_read` for CryEngine chunk
  structure, `dba_dump` for CAF/DBA animation channels, and `mannequin_dump` for
  fragment/scope metadata.
- Use the CLI, not MCP, for full export operations and end-to-end pipeline
  validation, e.g. `starbreaker entity export ...`.
- If the StarBreaker MCP server code changes, rebuild `starbreaker-mcp` and
  replace the deployed binary under `mcp/` after stopping the running MCP
  process. The client must be restarted to pick up the new server binary.
- On systems with Blender MCP, use it for generated `.blend` validation:
  inspect object hierarchy, transforms, linked libraries, missing files, world
  nodes, screenshots, and test renders directly in Blender. Use background
  Blender MCP file-summary tools for closed files, and execute Python in the
  connected Blender instance for live-scene checks. For native `.blend`
  transform, light, hierarchy, or render fixes, validate the generated file in
  Blender rather than relying only on JSON or Rust-side assertions.
- When driving Blender through MCP, reset scenes with
  `bpy.ops.wm.read_homefile(app_template="")`. Do not hand-delete objects,
  node groups, materials, or orphaned data. After addon edits, sync the live
  addon install before validating:

  ```bash
  rsync -a --delete blender_addon/starbreaker_addon/ \
    ~/.config/blender/5.1/scripts/addons/starbreaker_addon/
  ```

  Then purge cached `starbreaker_addon` modules, reset the scene, and re-enable
  the addon before validating.

## Key conventions

- No heuristics or hard-coded asset fixes in production code. Do not gate code
  on a specific ship, object name, mesh name, asset path, material name, item
  ID, or instance suffix. Avoid branches like `if name == ...` or
  `matches!(asset_path, ...)` for exporter behavior. Named assets are acceptable
  in regression tests, diagnostics, and repro scripts only; the production fix
  must derive a structural rule from Star Citizen/CryEngine data such as NMC
  node metadata, transform basis, geometry flags, shader family, blend mode,
  material XML, `.chrparams`, or DataCore records, then fix the whole category.
- New Rust source files must start with a `//!` module-doc header describing the
  file responsibility, important public types, and key functions. Update the
  header when the file's role changes.
- Use debug builds for normal Rust iteration. Release builds are for CI,
  deployable binaries, MCP server deployment, and final/export benchmark runs.
- The CLI package and binary are named `starbreaker`; do not refer to a
  `starbreaker-cli` package.
- For decomposed exports, the output argument is the shared export root that
  contains `Packages/` and `Data/`, not an individual package folder. Passing a
  package directory causes nested `Packages/.../Packages/...` output.
- Keep export contract changes backward-compatible and update
  `docs/decomposed-export-contract.md` when adding fields. Material naming and
  shader-family inputs are documented in
  `docs/blender-material-contract-naming-rules.md` and related docs.
- Do not run multiple cargo builds/tests in parallel against the same target
  directory; agents sharing the same build output can race.
