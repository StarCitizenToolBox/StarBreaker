# StarBreaker - AI Agent Instructions

This file covers the overall StarBreaker project (Rust crates, CLI, MCP
server, export pipeline). Blender-addon-specific guidance lives in
[blender_addon/AGENTS.md](blender_addon/AGENTS.md).

## Repository Layout

- `Cargo.toml` — workspace root. Crates live under `crates/`.
- `crates/starbreaker-3d/` — scene graph, `.soc` / light / interior
  parsing, decomposed-export JSON writer
  (`src/decomposed.rs`, `src/socpak.rs`, `src/types.rs`).
- `crates/starbreaker-p4k/`, `-chunks/`, `-cryxml/`, `-datacore/`,
  `-dds/`, `-wem/`, `-wwise/`, `-chf/`, `-common/` — format decoders.
- `cli/` — package name is `starbreaker` (binary `starbreaker`), NOT
  `starbreaker-cli`.
- `mcp/` — MCP server (see §MCP).
- `blender_addon/` — Python addon + tests. See its own AGENTS.md.
- `app/` — Tauri + React app (see `tasks.json` for build tasks).
- `docs/` — in-repo reference: export contract, material authoring,
  shader family inventory.

## Building

Use `cargo build` (debug) for iteration, NOT `cargo build --release`.
Debug profile is `[optimized + debuginfo]` in this workspace — fast
enough for testing. Release builds take much longer and are only
needed for deployment (MCP server, final binaries, CLI re-exports).

## Coding Practices

Shared across every language in the repo:

- **Keep files small.** A source file that grows past ~500 lines is a
  strong signal it wants to be split. Monolithic modules make diffs
  noisier, searches slower, and tests harder to target.
- **Split by responsibility, not by arbitrary line count.** Prefer
  cohesive modules (one type of concern per file) over sprawling
  "grab-bag" modules. Examples already in the tree:
  - Rust: each CryEngine format lives in its own crate under
    `crates/starbreaker-*`.
  - Python: `runtime/importer/` is split into mixins
    (`palette.py`, `decals.py`, `layers.py`, `materials.py`,
    `builders.py`, `groups.py`, `orchestration.py`) rather than one
    giant `importer.py`.
- **Fix the root cause, not the symptom.** Do not add `.max(small
  number)` style floors, fallback defaults, or try/except-pass around
  broken data. If the exporter is wrong, fix the exporter.
- **Never hard-code workarounds for specific assets.** Do not gate
  logic on a particular object name, mesh name, material name, ship
  name, texture path, socpak, item ID, or exact instance suffix. If one
  asset misbehaves, find the generic property of its category (NMC node
  metadata, transform basis, geometry flags, shader family, blend mode
  flag, alpha usage, etc.) and fix the rule for the whole category.
  Named-asset branches rot the moment upstream renames or adds
  siblings, and they are not acceptable production fixes.
- **Match existing conventions.** Read the neighbours before
  inventing a new pattern. Dataclass style in `manifest.py`, naming
  in `blender-material-contract-naming-rules.md`, error taxonomy in
  `starbreaker-common` — all of these exist so new code doesn't have
  to make them up again.
- **Don't over-engineer.** Only make changes that are directly
  requested or clearly necessary. Avoid speculative abstractions,
  "just in case" helpers, and refactors bundled into feature work.
- **Tests track behaviour, not lines.** Add or update tests when a
  behaviour changes; don't add tests just to bump coverage on
  trivial getters.
- **Every new Rust source file needs a `//!` module-doc header.** The
  first lines of every `.rs` file must be a `//!` block (one or more
  lines) describing the file's responsibility, key functions, and
  public types. This allows agents and developers to understand a
  file's purpose from its first line without reading it in full.
  When you significantly change a file's responsibility — adding a
  new group of functions, moving functions out, or changing the
  primary type it owns — update the `//!` header to reflect the
  current state. Stale headers mislead future readers.

## Testing

**TDD rule:** When a bug is found, write a failing test that reproduces it
*before* changing any code. Verify the test fails. Then fix the code.
Verify the test passes. This ensures tests are genuine regressions, not
written to match an already-known fix.

How to run tests:

```bash
# Rust — all workspace tests
cargo test --workspace

# Rust — targeted (faster during iteration)
cargo test -p starbreaker-3d --lib

# Blender addon (stubs bpy — runs on system Python)
cd blender_addon && python3 -m unittest discover -s tests -q
```

## Delegating Phases to Sub-Agents: Planning, Review, & Execution

When breaking down work into phases to delegate to sub-agents, use this structured approach to ensure quality, catch integration gaps early, and enable efficient parallel execution:

### 1. **Phase Decomposition & Dependency Tracking**

- **Break large work into phases** with clear, discrete deliverables (not vague umbrella tasks)
  - **Example**: "Phase 5: .blend assembly" → decompose into 5A (scene.blend), 5B (lights), 5C (empties), 5D (decals), each 1–2 days
  - Each sub-phase should have 1–3 clear functions/modules to add or modify
- **Map dependencies explicitly** in a structured format
  - **Options**: SQL `todo_deps` table, Markdown checklist in `plan.md`, spreadsheet, or even plain text comments in code
  - **Example dependencies**: 
    - Phase 5A must complete before 5B/5C (5A sets up collections; 5B/5C populate them)
    - 5B/5C can run sequentially to avoid cargo build race conditions
    - Phase 5D (decals) depends on 5C (empties) being done
  - **Key**: Make dependencies **visible and queryable** so agents know what blocks them
- **Avoid broad parallelism** for work that shares compilation targets
  - Multiple agents running `cargo build` simultaneously cause target-directory race conditions
  - **Solution**: Sequential execution (5A → 5B → 5C → 5D) is safer than parallel, even if slightly slower
  - If parallelism is needed, ensure agents don't share the same build toolchain and lock file

### 2. **Self-Review Checklist for Sub-Agents**

Before a sub-agent claims "done", it must **run its own code review** against a 10-point checklist:

```
□ Architecture: Functions fit cohesively into the pipeline; no unexpected dependencies
□ Integration: New code correctly calls/uses code from prior phases; no orphaned functions
□ Tests: ≥ 5–6 unit tests written; all pass; edge cases covered (empty lists, null refs, etc.)
□ Error handling: Invalid input is caught and reported clearly; no unwrap()/panic! on bad data
□ Conventions: Naming, layout, module headers (//! blocks), and style match the codebase
□ Regressions: All existing tests still pass; no previously-working code was broken
□ Performance: No gratuitous allocations, redundant loops, or O(n²) bugs introduced
□ Documentation: Function signatures have /// doc comments; public API is clear
□ Commit message: Clear, references phase number, includes Co-authored-by trailer
□ Final check: `cargo build --release` succeeds, `cargo test --workspace` all green
```

The agent should:
1. Run all tests locally before reporting completion
2. Verify the release build succeeds (not just debug)
3. Call out any checklist items that don't apply (e.g., "no performance work needed for Phase X")
4. Report all 10 items as green ✅ before saying "complete"

**Why this works**: Agents catch 80% of their own issues before hand-off. Self-review is faster than discovery-via-testing and builds confidence in the code.

### 3. **High-Level Code Review: Manager Perspective**

After the sub-agent completes, **do a high-level review** (5–10 minutes) focusing on:

- **✅ Architecture**: Does the new code fit cleanly into the pipeline? Are integration points correct?
- **✅ Test coverage**: Are there ≥ 5–6 tests? Do they cover the happy path and edge cases?
- **✅ Regressions**: Are all previously-passing tests still green? Did existing functionality break?
- **✅ Integration with prior phases**: Does this phase correctly use output from Phase N-1?
- **✅ No surprises**: Does the implementation match what was planned? Any scope creep or divergence?

**You are NOT** checking:
- Code style details (that's the agent's self-review)
- Performance micro-optimizations (only flag if egregiously bad)
- Every function signature (spot-check a few key ones)

**Report**: A one-paragraph summary ("Phase 5B approved: lights extract correctly, 3 new tests pass, integrates cleanly with Phase 5A") with any concerns flagged.

### 4. **Execution Flow**

1. **Plan Phase**: Write phase breakdown to `plan.md` or equivalent tracking doc; record dependencies
   - Use a format that suits your environment: Markdown checklist, SQL database, spreadsheet, or plain comments
   - **Key**: Dependencies must be visible so you can unblock Phase N+1 once Phase N completes
2. **Delegate Phase N**: Create sub-agent prompt with:
   - Phase description (what, why, where)
   - 10-point self-review checklist
   - ≥ 5–6 test requirement
   - Required: "Run self-review checklist before reporting done"
3. **Wait for Completion**: Agent runs → self-reviews → commits
4. **High-Level Review**: Read agent output; run tests locally; spot-check integration; approve/flag issues
5. **Mark Done & Unlock Next**: Update your tracking doc (plan.md, SQL, spreadsheet, etc.) to mark Phase N complete and Phase N+1 unblocked
6. **Delegate Phase N+1**: Repeat

### 5. **Example: Phase 5 Delegation (Real Data)**

**What Was Done**:
- Phase 5A (Scene assembly): Delegated, 7 tests delivered ✅
- Phase 5B (Light hierarchy): Delegated after 5A, 3 tests ✅
- Phase 5C (Empty hierarchy): Delegated after 5B, 11 tests ✅
- Phase 5D (Decals): Delegated after 5C, 5 tests ✅
- **Cumulative**: 268 tests passing, no regressions, no post-review fixes

**Tracking Used**: SQL `todos` table with `status` (pending/in_progress/done) and `todo_deps` for phase dependencies. But the same pattern works with:
- **Markdown**: Checklist in `plan.md` with ✅/🔄/⏳ status indicators
- **Spreadsheet**: Google Sheets or Excel with phase/status/dependency columns
- **Simple comments**: In-code comments marking each phase as `// Phase 5A: COMPLETE`, `// Phase 5B: in progress`, etc.

**Why It Worked**:
- Clear handoff (plan → checklist → code → self-review → high-level approval)
- Sequential execution (no build conflicts)
- Early detection of integration gaps (Phase 5A had stubs that broke 5B; caught immediately, fixed within Phase 5C commit)
- Tests validated integration end-to-end before Phase 6 validation started

### When to Use This Process

- **Large multi-phase features** (Phases 5–6, architecture refactors, pipeline rewrites)
- **Parallel sub-agent work** (break into independent sub-phases + dependencies)
- **Quality-critical paths** (export pipeline, data integrity, user-facing features)
- **Not needed for** small bugfixes, docs updates, or single-file changes

---

## Troubleshooting

When a bug is found or something behaves unexpectedly:

- **Find the root cause, fix that.** Do not paper over symptoms with
  clamp floors, fallback defaults, `.unwrap_or(0)` silencers, or
  try/except-pass. If the data is wrong, trace back to where it is
  written or parsed and fix it there.
- **No hard-coding, no heuristics.** Do not gate logic on a
  specific object name, mesh name, asset name, ship name, material
  path, exact instance suffix, or magic number. Discover the structural
  property (NMC node metadata, transform basis, geometry flags, shader
  family, blend mode flag, alpha channel usage, chrparams event type,
  ...) and fix the rule for the whole category. If the structural rule
  is not known yet, stop and investigate it; do not commit a
  name-matched workaround as a "temporary" fix.
- **Ask: how does the game engine handle this?** Star Citizen runs on
  Star Engine, Cloud Imperium's fork of CryEngine / Lumberyard. It
  shares most CryEngine conventions but diverges in places. When the
  fix is ambiguous — a channel remapping, a coordinate space, a
  material slot ordering, a light unit — look at how Star Engine
  would process the same data. The canonical source is the
  `.chrparams`, `.dba`, `.mtl`, and shader definitions extracted from
  `Data.p4k`. Mirroring the engine's own logic is almost always more
  correct and more robust than a derived heuristic.

## Python

Always use `uv run python` instead of `python`, `python3`, or `py`
when running Python scripts or one-liners. This project uses `uv` for
Python tooling.

Exception: the Blender addon test suite runs with the system
`python3` (see `blender_addon/AGENTS.md`), because it stubs `bpy`.

## CLI re-export

After changing the Rust exporter, re-export a ship and reimport it in
Blender to verify behaviour. The binary is `target/release/starbreaker`
(package name `starbreaker`, not `starbreaker-cli`). Invoke it with:

```bash
SC_DATA_P4K=<path to Data.p4k> \
  ./target/release/starbreaker entity export <entity_name> <export_root> \
  --kind decomposed
```

`--kind decomposed` emits the reusable `scene.json` +
`Packages/<name>/` layout documented in
`docs/decomposed-export-contract.md`. Workspace-specific ship paths
and the `SC_DATA_P4K` location are in the workspace-root AGENTS.md.

## Git

The StarBreaker repo is self-contained (root = `StarBreaker/`); the
parent workspace is not a git repo. Commit with whatever author identity is already configured in your
environment. If git complains that `user.name` / `user.email` are
unset, configure them via `git config` rather than inlining `-c
user.name=... -c user.email=...` on every commit — doing that leaks
whatever placeholder you happen to use into the repo history.

## MCP Server

The StarBreaker MCP server provides DataCore, P4k, and chunk
inspection tools for Claude Code. To deploy after making changes:

```bash
# Windows
taskkill //F //IM starbreaker-mcp.exe 2>/dev/null; cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp.exe mcp/starbreaker-mcp.exe

# Linux
pkill -f starbreaker-mcp || true
cargo build --release -p starbreaker-mcp && cp target/release/starbreaker-mcp mcp/starbreaker-mcp
```

You must kill the running MCP process before copying, or the file
will be locked / busy. Then restart the client to pick up the new
binary. The `.mcp.json` points to the deployed copy, not the build
artifact, so the running server isn't locked by workspace builds.

### When to Add MCP Tools

If you find yourself doing a task that MCP would be a good fit for
(e.g., repeatedly querying game data, inspecting files, or doing
lookups that shell commands are awkward for), add a new tool to the
MCP server or note it as a task for later.

### Available MCP Tools

Use these tools (via ToolSearch for `starbreaker`) to research game
data without shelling out to the CLI:

- **`search_entities`** — find EntityClassDefinition records by name substring
- **`search_records`** — search ALL DataCore record types (tint palettes, ammo, attachables, etc.)
- **`entity_loadout`** — dump resolved loadout tree (processed — resolves entity references and geometry paths)
- **`datacore_record`** — dump full record as JSON (by GUID or name substring)
- **`datacore_query`** — query a specific property path (e.g. `Components[VehicleComponentParams].vehicleDefinition`)
- **`p4k_list`** — browse P4k directories (shows size, compression, encryption)
- **`p4k_read`** — read P4k files (auto-decodes CryXML to XML text)
- **`image_preview`** — decode and view DDS/PNG/JPG textures from P4k (multimodal — you can see the image)
- **`chunk_list`** — list IVO/CrCh chunks in geometry files (type, version, size, NMC node summary)
- **`chunk_read`** — hex dump of specific chunks

### When to Use MCP vs CLI

- **MCP tools** return raw/lightly-processed data for research. Use
  them to investigate DataCore records, browse files, inspect
  textures, and understand game data structure.
- **CLI** (`cargo run --bin starbreaker` or the release binary
  above) is for export operations and testing the full export
  pipeline. Use it when you need to actually export a GLB or test
  changes to the export code.
- For raw DataCore loadout data, use `datacore_query` with path
  `Components[SEntityComponentDefaultLoadoutParams]`. The
  `entity_loadout` tool returns StarBreaker's processed/resolved tree
  instead.

## Large Source Files — Decomposition Plans

The two previously-monolithic source files have been fully decomposed:

- **`crates/starbreaker-3d/src/pipeline/`** (Phase 51 ✅ commit `389880a`) — 10 sub-modules
- **`crates/starbreaker-3d/src/animation/`** (Phase 59 ✅ commit `efcca33`) — 9 sub-modules

Every sub-module has a `//!` header. To find where a function lives, use
`grep_search` for the function name, or `file_search` + `read_file` on
the `//!` first line of each file in the module directory.

## Reference Docs

- `docs/decomposed-export-contract.md` — scene.json / palettes.json /
  liveries.json / material-sidecar contract. Update when adding new
  fields to the exporter.
- `docs/blender-material-contract-naming-rules.md` — how shader
  families and slots are named and reconstructed.
- `docs/blender-material-slot-evidence.md` — evidence dumps used to
  derive the naming rules.
- `docs/blender-material-template-authoring.md` — how to author
  reusable Blender material node templates.
- `docs/blender-shader-family-inventory.json` — the canonical list of
  CryEngine shader families we know about.
