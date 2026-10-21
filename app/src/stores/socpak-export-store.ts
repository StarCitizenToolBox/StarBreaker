import { create } from "zustand";
import { persist } from "zustand/middleware";
import { tauriStorage } from "../lib/tauri-storage";

interface SocpakExportState {
  optionsWidth: number;
  search: string;
  lod: number;
  mip: number;
  materialMode: string;
  includeLights: boolean;
  overwriteExistingAssets: boolean;
  includeNodraw: boolean;
  outputDir: string | null;
  setOptionsWidth: (v: number) => void;
  setSearch: (v: string) => void;
  setLod: (v: number) => void;
  setMip: (v: number) => void;
  setMaterialMode: (v: string) => void;
  setIncludeLights: (v: boolean) => void;
  setOverwriteExistingAssets: (v: boolean) => void;
  setIncludeNodraw: (v: boolean) => void;
  setOutputDir: (v: string | null) => void;
}

// `search` is intentionally excluded: persisting it silently re-applies a
// stale filter on launch (which also hides most containers and breaks the
// default-collapsed grouping). It stays runtime-only, defaulting to "".
type PersistedSocpakExportState = Pick<
  SocpakExportState,
  | "optionsWidth"
  | "lod"
  | "mip"
  | "materialMode"
  | "includeLights"
  | "overwriteExistingAssets"
  | "includeNodraw"
  | "outputDir"
>;

export const useSocpakExportStore = create<SocpakExportState>()(
  persist<SocpakExportState, [], [], PersistedSocpakExportState>(
    (set) => ({
      optionsWidth: 280,
      search: "",
      lod: 1,
      mip: 2,
      materialMode: "textures",
      includeLights: true,
      overwriteExistingAssets: true,
      includeNodraw: false,
      outputDir: null,
      setOptionsWidth: (v) => set({ optionsWidth: v }),
      setSearch: (v) => set({ search: v }),
      setLod: (v) => set({ lod: v }),
      setMip: (v) => set({ mip: v }),
      setMaterialMode: (v) => set({ materialMode: v }),
      setIncludeLights: (v) => set({ includeLights: v }),
      setOverwriteExistingAssets: (v) => set({ overwriteExistingAssets: v }),
      setIncludeNodraw: (v) => set({ includeNodraw: v }),
      setOutputDir: (v) => set({ outputDir: v }),
    }),
    {
      name: "socpak-export",
      storage: tauriStorage,
      // Bump forces `migrate` to run so any previously-persisted `search`
      // value (e.g. "hangar") is dropped from existing stores.
      version: 1,
      migrate: (persisted) => {
        const state = { ...(persisted as Record<string, unknown>) };
        delete state.search;
        return state as unknown as PersistedSocpakExportState;
      },
      partialize: (s) => ({
        optionsWidth: s.optionsWidth,
        lod: s.lod,
        mip: s.mip,
        materialMode: s.materialMode,
        includeLights: s.includeLights,
        overwriteExistingAssets: s.overwriteExistingAssets,
        includeNodraw: s.includeNodraw,
        outputDir: s.outputDir,
      }),
    },
  ),
);
