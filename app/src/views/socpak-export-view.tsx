import { useEffect, useMemo, useState } from "react";
import { ResizeHandle } from "../components/resize-handle";
import { SocpakSelectionDialog } from "../components/socpak-selection-dialog";
import { useSocpakExportStore } from "../stores/socpak-export-store";
import {
  browseOutputDir,
  exportSocpaks,
  inspectSocpakHierarchy,
  onSocpakExportProgress,
  scanSocpaks,
  type SocpakDto,
  type SocpakExportDone,
  type SocpakExportProgress,
  type SocpakExportRequest,
  type SocpakHierarchyNode,
} from "../lib/commands";

// Display order for the top-level groups produced by the Rust categoriser
// (`socpak_category::categorize_socpak`). Categories not listed here (should
// only be future additions) sort to the end, alphabetically.
const CATEGORY_ORDER = [
  "Cities & Landing Zones",
  "Space Stations",
  "Outposts & Surface Bases",
  "Underground & Caves",
  "Derelicts & Wrecks",
  "Planet & System Set-Dressing",
  "Hangars",
  "Shops & Interiors",
  "Ships",
  "Ground Vehicles",
  "Gameplay Setup",
  "Shared Modules & Lighting",
  "Props, Flair & Decor",
  "Game Modes & Test Maps",
  "Locations — Other",
  "Other",
];

interface SocpakSubGroup {
  name: string;
  entries: SocpakDto[];
}

interface SocpakGroup {
  name: string;
  count: number;
  subs: SocpakSubGroup[];
}

export function SocpakExportView() {
  const optionsWidth = useSocpakExportStore((s) => s.optionsWidth);
  const search = useSocpakExportStore((s) => s.search);
  const lod = useSocpakExportStore((s) => s.lod);
  const mip = useSocpakExportStore((s) => s.mip);
  const materialMode = useSocpakExportStore((s) => s.materialMode);
  const includeLights = useSocpakExportStore((s) => s.includeLights);
  const overwriteExistingAssets = useSocpakExportStore((s) => s.overwriteExistingAssets);
  const includeNodraw = useSocpakExportStore((s) => s.includeNodraw);
  const outputDir = useSocpakExportStore((s) => s.outputDir);
  const setOptionsWidth = useSocpakExportStore((s) => s.setOptionsWidth);
  const setSearch = useSocpakExportStore((s) => s.setSearch);
  const setLod = useSocpakExportStore((s) => s.setLod);
  const setMip = useSocpakExportStore((s) => s.setMip);
  const setMaterialMode = useSocpakExportStore((s) => s.setMaterialMode);
  const setIncludeLights = useSocpakExportStore((s) => s.setIncludeLights);
  const setOverwriteExistingAssets = useSocpakExportStore((s) => s.setOverwriteExistingAssets);
  const setIncludeNodraw = useSocpakExportStore((s) => s.setIncludeNodraw);
  const setOutputDir = useSocpakExportStore((s) => s.setOutputDir);

  const [socpaks, setSocpaks] = useState<SocpakDto[]>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [exporting, setExporting] = useState(false);
  const [result, setResult] = useState<SocpakExportDone | null>(null);
  const [progress, setProgress] = useState<SocpakExportProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [hierarchyOpen, setHierarchyOpen] = useState(false);
  const [hierarchyNodes, setHierarchyNodes] = useState<SocpakHierarchyNode[]>([]);
  const [selectedHierarchyPaths, setSelectedHierarchyPaths] = useState<Set<string>>(new Set());
  const [openCats, setOpenCats] = useState<Set<string>>(new Set());
  const [openSubs, setOpenSubs] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setLoading(true);
      scanSocpaks(search)
        .then((items) => {
          if (!cancelled) {
            setSocpaks(items);
            setLoading(false);
          }
        })
        .catch((err) => {
          if (!cancelled) {
            setError(String(err));
            setLoading(false);
          }
        });
    }, 180);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [search]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    onSocpakExportProgress((next) => {
      if (!cancelled) {
        setProgress(next);
        if (next.error) {
          setError(next.error);
        }
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const visiblePaths = useMemo(() => socpaks.map((entry) => entry.path), [socpaks]);
  const selectedVisibleCount = visiblePaths.filter((path) => selected.has(path)).length;

  // Two-tier grouping (category -> subcategory -> entries) driven entirely by
  // the category fields the backend derives from each socpak's path.
  const groups = useMemo<SocpakGroup[]>(() => {
    const byCategory = new Map<string, Map<string, SocpakDto[]>>();
    for (const entry of socpaks) {
      let subs = byCategory.get(entry.category);
      if (!subs) {
        subs = new Map();
        byCategory.set(entry.category, subs);
      }
      const subName = entry.subcategory || "General";
      const bucket = subs.get(subName);
      if (bucket) bucket.push(entry);
      else subs.set(subName, [entry]);
    }
    const orderIndex = (name: string) => {
      const i = CATEGORY_ORDER.indexOf(name);
      return i === -1 ? CATEGORY_ORDER.length : i;
    };
    return Array.from(byCategory.entries())
      .map(([name, subsMap]) => {
        const subs = Array.from(subsMap.entries())
          .map(([subName, entries]) => ({ name: subName, entries }))
          .sort((a, b) => a.name.localeCompare(b.name));
        const count = subs.reduce((total, sub) => total + sub.entries.length, 0);
        return { name, count, subs };
      })
      .sort((a, b) => orderIndex(a.name) - orderIndex(b.name) || a.name.localeCompare(b.name));
  }, [socpaks]);

  // Groups are collapsed by default. A live search auto-expands the matching
  // groups (the backend has already filtered `socpaks` to matches); clearing
  // the search collapses everything again. Between those transitions, manual
  // toggles are the sole source of truth, so collapsing always works.
  useEffect(() => {
    if (search.trim().length === 0) {
      setOpenCats(new Set());
      setOpenSubs(new Set());
      return;
    }
    setOpenCats(new Set(groups.map((group) => group.name)));
    setOpenSubs(
      new Set(groups.flatMap((group) => group.subs.map((sub) => `${group.name}||${sub.name}`))),
    );
  }, [search, groups]);

  const countSelected = (entries: SocpakDto[]) =>
    entries.reduce((total, entry) => total + (selected.has(entry.path) ? 1 : 0), 0);

  const toggleCat = (name: string) => {
    setOpenCats((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const toggleSub = (key: string) => {
    setOpenSubs((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const setGroupSelected = (entries: SocpakDto[], select: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      for (const entry of entries) {
        if (select) next.add(entry.path);
        else next.delete(entry.path);
      }
      return next;
    });
  };
  const canExport = selected.size > 0 && outputDir !== null && !exporting && !inspecting;
  const allDone = progress !== null && progress.total > 0 && progress.current >= progress.total;
  const progressPercent = allDone ? 100 : Math.min(Math.round((progress?.fraction ?? 0) * 100), 99);
  const progressBarFraction = allDone ? 1 : Math.min(progress?.fraction ?? 0, 0.99);
  const activeSocpakName = progress?.socpak_path.split(/[\\/]/).pop() ?? "";

  const collectSocpakPaths = (nodes: SocpakHierarchyNode[], out: string[] = []) => {
    for (const node of nodes) {
      out.push(node.path);
      collectSocpakPaths(node.children, out);
    }
    return out;
  };

  const findAncestorPaths = (
    targetPath: string,
    nodes: SocpakHierarchyNode[],
    ancestors: string[] = [],
  ): string[] | null => {
    for (const node of nodes) {
      const nextAncestors = [...ancestors, node.path];
      if (node.path === targetPath) return nextAncestors;
      const found = findAncestorPaths(targetPath, node.children, nextAncestors);
      if (found) return found;
    }
    return null;
  };

  const hierarchyAncestorsFor = (path: string) =>
    findAncestorPaths(path, hierarchyNodes) ?? [path];

  const toggleSocpak = (path: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const selectVisible = () => {
    setSelected((current) => {
      const next = new Set(current);
      for (const path of visiblePaths) next.add(path);
      return next;
    });
  };

  const clearVisible = () => {
    setSelected((current) => {
      const next = new Set(current);
      for (const path of visiblePaths) next.delete(path);
      return next;
    });
  };

  const browseOutput = () => {
    browseOutputDir().then((dir) => {
      if (dir !== null) setOutputDir(dir);
    });
  };

  const startSocpakExport = (request: SocpakExportRequest) => {
    setExporting(true);
    setResult(null);
    setProgress({
      current: 0,
      total: selected.size,
      fraction: 0,
      socpak_path: "",
      package_name: "",
      stage: "Preparing export",
      files_written: 0,
      files_total: 0,
      error: null,
    });
    setError(null);
    exportSocpaks(request)
      .then((done) => setResult(done))
      .catch((err) => setError(String(err)))
      .finally(() => setExporting(false));
  };

  const buildExportRequest = (pathFilter: string[] | null = null): SocpakExportRequest => ({
    socpak_paths: Array.from(selected),
    output_dir: outputDir!,
    lod,
    mip,
    material_mode: materialMode,
    include_lights: includeLights,
    overwrite_existing_assets: overwriteExistingAssets,
    include_nodraw: includeNodraw,
    socpak_path_filter: pathFilter,
  });

  const runExport = () => {
    const rootPaths = Array.from(selected);
    setInspecting(true);
    setError(null);
    inspectSocpakHierarchy({ socpak_paths: rootPaths })
      .then((nodes) => {
        // `inspect_socpak_hierarchy` skips socpaks it cannot inspect, so a
        // partial result means some selected socpaks have no hierarchy. The
        // sub-container path filter is only meaningful for a fully-inspected
        // tree; applying it while some roots are missing would silently drop
        // those roots from the export. When the result is incomplete, fall back
        // to exporting the full selection unfiltered, and surface why so the
        // skip is never silent.
        if (nodes.length < rootPaths.length) {
          if (nodes.length > 0) {
            setError(
              `Could not inspect ${rootPaths.length - nodes.length} of ${rootPaths.length} ` +
                `selected socpak(s); exporting the full selection without sub-container filtering.`,
            );
          }
          startSocpakExport(buildExportRequest());
          return;
        }
        setHierarchyNodes(nodes);
        setSelectedHierarchyPaths(new Set(collectSocpakPaths(nodes)));
        setHierarchyOpen(true);
      })
      .catch((err) => setError(String(err)))
      .finally(() => setInspecting(false));
  };

  const toggleHierarchyNode = (node: SocpakHierarchyNode, checked: boolean) => {
    const paths = collectSocpakPaths([node]);
    setSelectedHierarchyPaths((current) => {
      const next = new Set(current);
      for (const path of paths) {
        if (checked) {
          for (const ancestorPath of hierarchyAncestorsFor(path)) {
            next.add(ancestorPath);
          }
          next.add(path);
        } else {
          next.delete(path);
        }
      }
      return next;
    });
  };

  const confirmHierarchySelection = () => {
    setHierarchyOpen(false);
    startSocpakExport(buildExportRequest(Array.from(selectedHierarchyPaths)));
  };

  return (
    <div className="flex-1 flex overflow-hidden relative">
      {exporting && (
        <div className="absolute inset-0 z-10 bg-bg/80 backdrop-blur-sm flex items-center justify-center">
          <div className="w-[420px] bg-bg-alt border border-border rounded-lg p-6 flex flex-col gap-4 shadow-lg">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-text">Exporting socpak packages</h3>
                <p className="text-[11px] text-text-dim truncate mt-1">
                  {activeSocpakName || "Preparing export"}
                </p>
              </div>
              <div className="text-right shrink-0">
                <p className="text-xs text-text tabular-nums">{progressPercent}%</p>
                <p className="text-[10px] text-text-faint tabular-nums mt-0.5">
                  {progress?.current ?? 0}/{progress?.total ?? selected.size}
                </p>
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <div className="w-full bg-surface rounded-full h-2 overflow-hidden">
                <div
                  className="bg-accent h-full rounded-full transition-all duration-300"
                  style={{ width: `${progressBarFraction * 100}%` }}
                />
              </div>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <p className="text-[11px] text-text-dim truncate">
                    {progress?.stage ?? "Preparing export"}
                  </p>
                  {progress?.package_name && (
                    <p className="text-[10px] text-text-faint truncate mt-0.5">
                      Package: {progress.package_name}
                    </p>
                  )}
                </div>
                {progress && progress.files_total > 0 && (
                  <p className="text-[10px] text-text-faint tabular-nums shrink-0">
                    {progress.files_written}/{progress.files_total} files
                  </p>
                )}
                {progress && progress.files_total === 0 && progress.files_written > 0 && (
                  <p className="text-[10px] text-text-faint tabular-nums shrink-0">
                    {progress.files_written} container{progress.files_written === 1 ? "" : "s"}
                  </p>
                )}
              </div>
            </div>

            {progress?.socpak_path && (
              <p className="text-[10px] text-text-faint leading-relaxed break-all">
                {progress.socpak_path}
              </p>
            )}
            {error && (
              <div className="max-h-24 overflow-y-auto rounded bg-danger/5 border border-danger/20 px-3 py-2">
                <p className="text-[11px] text-danger/80 leading-relaxed break-words">
                  {error}
                </p>
              </div>
            )}
          </div>
        </div>
      )}

      <div className="flex-1 flex flex-col min-w-0">
        <div
          className="flex items-center gap-2 px-3 border-b border-border bg-bg-alt shrink-0"
          style={{ height: "var(--toolbar-height)" }}
        >
          <input
            type="text"
            placeholder="Search socpak paths..."
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="flex-1 bg-surface rounded-md px-3 py-1.5 text-sm text-text placeholder:text-text-faint outline-none focus:ring-1 focus:ring-ring"
          />
          <button
            onClick={selectVisible}
            className="text-[11px] text-text-dim hover:text-text px-2 py-1 rounded-md hover:bg-surface/60 transition-colors cursor-pointer shrink-0"
          >
            All
          </button>
          <button
            onClick={clearVisible}
            className="text-[11px] text-text-dim hover:text-text px-2 py-1 rounded-md hover:bg-surface/60 transition-colors cursor-pointer shrink-0"
          >
            None
          </button>
          <span className="text-[11px] text-text-faint tabular-nums shrink-0">
            {selectedVisibleCount}/{socpaks.length}
          </span>
        </div>

        <div className="flex-1 overflow-y-auto px-1">
          {loading && (
            <p className="px-3 py-3 text-xs text-text-dim">Scanning socpaks...</p>
          )}
          {!loading && socpaks.length === 0 && (
            <p className="px-3 py-3 text-xs text-text-dim">No socpaks match this filter.</p>
          )}
          {!loading && groups.map((group) => {
            const catOpen = openCats.has(group.name);
            const catSelected = group.subs.reduce((total, sub) => total + countSelected(sub.entries), 0);
            return (
              <div key={group.name} className="mb-px">
                <div className="flex items-center gap-1 group/cat">
                  <button
                    onClick={() => toggleCat(group.name)}
                    className="flex-1 min-w-0 flex items-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-medium text-text hover:bg-surface/50 transition-colors cursor-pointer"
                  >
                    <span className="text-text-faint w-3 shrink-0 text-center">{catOpen ? "▾" : "▸"}</span>
                    <span className="truncate">{group.name}</span>
                    {catSelected > 0 && (
                      <span className="text-[10px] text-accent tabular-nums shrink-0">{catSelected}</span>
                    )}
                    <span className="ml-auto text-[10px] text-text-faint tabular-nums shrink-0">{group.count}</span>
                  </button>
                  {catOpen && (
                    <button
                      onClick={() => setGroupSelected(group.subs.flatMap((sub) => sub.entries), catSelected < group.count)}
                      className="text-[10px] text-text-faint hover:text-text px-1.5 py-1 rounded opacity-0 group-hover/cat:opacity-100 transition-opacity cursor-pointer shrink-0"
                      title={catSelected < group.count ? "Select all in category" : "Clear category"}
                    >
                      {catSelected < group.count ? "All" : "None"}
                    </button>
                  )}
                </div>

                {catOpen && group.subs.map((sub) => {
                  const subKey = `${group.name}||${sub.name}`;
                  const subOpen = openSubs.has(subKey);
                  const subSelected = countSelected(sub.entries);
                  return (
                    <div key={subKey} className="ml-3">
                      <button
                        onClick={() => toggleSub(subKey)}
                        className="w-full flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] text-text-sub hover:bg-surface/40 transition-colors cursor-pointer"
                      >
                        <span className="text-text-faint w-3 shrink-0 text-center">{subOpen ? "▾" : "▸"}</span>
                        <span className="truncate">{sub.name}</span>
                        {subSelected > 0 && (
                          <span className="text-[10px] text-accent tabular-nums shrink-0">{subSelected}</span>
                        )}
                        <span className="ml-auto text-[10px] text-text-faint tabular-nums shrink-0">{sub.entries.length}</span>
                      </button>

                      {subOpen && sub.entries.map((entry) => {
                        const isSelected = selected.has(entry.path);
                        const base = entry.path.split(/[\\/]/).pop() ?? entry.path;
                        return (
                          <label
                            key={entry.path}
                            className={`flex items-center gap-2.5 pl-7 pr-3 py-[4px] rounded-md cursor-pointer text-xs transition-colors select-none ${
                              isSelected ? "bg-primary/8 text-text" : "text-text-sub hover:bg-surface/40"
                            }`}
                            title={entry.path}
                          >
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleSocpak(entry.path)}
                              className="accent-accent w-3.5 h-3.5 rounded shrink-0"
                            />
                            <span className="truncate">{base}</span>
                          </label>
                        );
                      })}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>

      <ResizeHandle width={optionsWidth} onResize={setOptionsWidth} side="left" min={220} max={420} />
      <div className="shrink-0 border-l border-border bg-bg-alt flex flex-col" style={{ width: optionsWidth }}>
        <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-5">
          <h2 className="text-xs font-semibold text-primary uppercase tracking-wider">
            Socpak Export
          </h2>

          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-text-sub">LOD Level</span>
              <span className="text-xs text-text-faint tabular-nums">{lod}</span>
            </div>
            <input
              type="range"
              min={0}
              max={4}
              value={lod}
              onChange={(event) => setLod(Number(event.target.value))}
              className="w-full accent-accent h-1.5"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-text-sub">Texture Mip</span>
              <span className="text-xs text-text-faint tabular-nums">{mip}</span>
            </div>
            <input
              type="range"
              min={0}
              max={6}
              value={mip}
              onChange={(event) => setMip(Number(event.target.value))}
              className="w-full accent-accent h-1.5"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <span className="text-xs text-text-sub">Materials</span>
            <div className="flex flex-col gap-1">
              {([
                ["none", "None"],
                ["colors", "Colors"],
                ["textures", "Textures"],
                ["all", "All (experimental)"],
              ] as const).map(([value, label]) => (
                <label key={value} className="flex items-center gap-2 cursor-pointer group">
                  <input
                    type="radio"
                    name="socpakMaterialMode"
                    value={value}
                    checked={materialMode === value}
                    onChange={() => setMaterialMode(value)}
                    className="accent-accent w-3 h-3"
                  />
                  <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                    {label}
                  </span>
                </label>
              ))}
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeLights}
                onChange={(event) => setIncludeLights(event.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include lights
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={overwriteExistingAssets}
                onChange={(event) => setOverwriteExistingAssets(event.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Overwrite existing assets
              </span>
            </label>
            <label className="flex items-center gap-2.5 cursor-pointer group">
              <input
                type="checkbox"
                checked={includeNodraw}
                onChange={(event) => setIncludeNodraw(event.target.checked)}
                className="accent-accent w-3.5 h-3.5 rounded"
              />
              <span className="text-xs text-text-sub group-hover:text-text transition-colors">
                Include NoDraw faces and sidecars
              </span>
            </label>
          </div>

          <div className="flex flex-col gap-1.5">
            <span className="text-xs text-text-sub">Output directory</span>
            <button
              onClick={browseOutput}
              className="flex items-center gap-2 w-full bg-surface/50 border border-border rounded-md px-3 py-2 text-xs text-left cursor-pointer hover:bg-surface/80 transition-colors"
            >
              <span className={outputDir ? "text-text truncate" : "text-text-faint"}>
                {outputDir ?? "Choose folder..."}
              </span>
            </button>
          </div>
        </div>

        <div className="p-4 border-t border-border flex flex-col gap-3">
          {result && (
            <p className="text-[11px] text-success leading-relaxed">
              Wrote {result.file_count} files into {result.package_names.length} package{result.package_names.length === 1 ? "" : "s"}.
            </p>
          )}
          {error && (
            <p className="text-[11px] text-danger leading-relaxed break-words">{error}</p>
          )}
          <button
            onClick={runExport}
            disabled={!canExport}
            className={`w-full py-2 rounded-md text-xs font-medium transition-colors cursor-pointer ${
              canExport
                ? "bg-accent text-on-accent hover:brightness-110"
                : "bg-surface text-text-faint cursor-not-allowed"
            }`}
          >
            {selected.size === 0
              ? "Select socpaks to export"
              : outputDir === null
                ? "Choose output directory"
                : inspecting
                  ? "Crawling hierarchy..."
                : `Export ${selected.size} socpak${selected.size === 1 ? "" : "s"}`}
          </button>
        </div>
      </div>

      {hierarchyOpen && (
        <SocpakSelectionDialog
          nodes={hierarchyNodes}
          selectedPaths={selectedHierarchyPaths}
          busy={exporting}
          onToggle={toggleHierarchyNode}
          onSelectAll={() => setSelectedHierarchyPaths(new Set(collectSocpakPaths(hierarchyNodes)))}
          onClearAll={() => setSelectedHierarchyPaths(new Set())}
          onConfirm={confirmHierarchySelection}
          onCancel={() => setHierarchyOpen(false)}
        />
      )}
    </div>
  );
}
