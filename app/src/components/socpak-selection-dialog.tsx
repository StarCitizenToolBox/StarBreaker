import { ChevronDown, ChevronRight, X } from "lucide-react";
import { useMemo, useState } from "react";
import type { SocpakHierarchyNode } from "../lib/commands";

interface SocpakSelectionDialogProps {
  nodes: SocpakHierarchyNode[];
  selectedPaths: Set<string>;
  busy: boolean;
  onToggle: (node: SocpakHierarchyNode, checked: boolean) => void;
  onSelectAll: () => void;
  onClearAll: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}

function collectPaths(nodes: SocpakHierarchyNode[], out: string[] = []): string[] {
  for (const node of nodes) {
    out.push(node.path);
    collectPaths(node.children, out);
  }
  return out;
}

function NodeRow({
  node,
  selectedPaths,
  onToggle,
}: {
  node: SocpakHierarchyNode;
  selectedPaths: Set<string>;
  onToggle: (node: SocpakHierarchyNode, checked: boolean) => void;
}) {
  const [expanded, setExpanded] = useState(true);
  const hasChildren = node.children.length > 0;
  const subtreePaths = useMemo(() => collectPaths([node]), [node]);
  const selectedCount = subtreePaths.filter((path) => selectedPaths.has(path)).length;
  const checked = selectedPaths.has(node.path);
  const indeterminate = selectedCount > 0 && selectedCount < subtreePaths.length;

  return (
    <div>
      <div
        className={`grid grid-cols-[22px_22px_minmax(0,1fr)_auto] items-center gap-2 px-2 py-1.5 rounded-md text-xs ${
          checked
            ? "bg-primary/8 text-text"
            : indeterminate
              ? "bg-warning/8 text-text-sub"
              : "text-text-sub hover:bg-surface/50"
        }`}
        style={{ marginLeft: node.depth * 18 }}
      >
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="w-[22px] h-[22px] flex items-center justify-center rounded hover:bg-surface-hi text-text-dim disabled:opacity-0"
          disabled={!hasChildren}
          title={expanded ? "Collapse" : "Expand"}
        >
          {hasChildren && (expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />)}
        </button>
        <input
          type="checkbox"
          checked={checked}
          ref={(input) => {
            if (input) input.indeterminate = indeterminate;
          }}
          onChange={(event) => onToggle(node, event.target.checked)}
          className="accent-accent w-3.5 h-3.5 rounded"
        />
        <div className="min-w-0">
          <div className="flex items-center gap-2 min-w-0">
            <span className="truncate text-text">{node.name}</span>
            {node.entity_name && (
              <span className="truncate text-[10px] text-text-faint">
                {node.entity_name}
              </span>
            )}
          </div>
          <p className="text-[10px] text-text-faint truncate mt-0.5">{node.path}</p>
        </div>
        <div className="flex items-center gap-2 text-[10px] text-text-faint tabular-nums">
          <span>{node.mesh_count} mesh</span>
          <span>{node.light_count} light</span>
        </div>
      </div>
      {expanded && node.children.length > 0 && (
        <div className="mt-0.5">
          {node.children.map((child) => (
            <NodeRow
              key={`${child.path}-${child.depth}-${child.entity_name ?? ""}`}
              node={child}
              selectedPaths={selectedPaths}
              onToggle={onToggle}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function SocpakSelectionDialog({
  nodes,
  selectedPaths,
  busy,
  onToggle,
  onSelectAll,
  onClearAll,
  onConfirm,
  onCancel,
}: SocpakSelectionDialogProps) {
  const allPaths = useMemo(() => collectPaths(nodes), [nodes]);
  const selectedCount = allPaths.filter((path) => selectedPaths.has(path)).length;

  return (
    <div className="fixed inset-0 bg-bg/80 backdrop-blur-sm flex items-center justify-center z-50">
      <div className="bg-bg-alt border border-border rounded-lg shadow-lg w-[860px] max-w-[92vw] max-h-[86vh] flex flex-col overflow-hidden">
        <div className="px-4 py-3 border-b border-border flex items-center justify-between gap-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-text">Select socpak parts</h3>
            <p className="text-[11px] text-text-dim mt-0.5">
              {selectedCount}/{allPaths.length} containers selected
            </p>
          </div>
          <button
            type="button"
            onClick={onCancel}
            className="w-8 h-8 rounded-md flex items-center justify-center text-text-dim hover:text-text hover:bg-surface/70"
            title="Close"
          >
            <X size={16} />
          </button>
        </div>

        <div className="px-4 py-2 border-b border-border flex items-center gap-2">
          <button
            type="button"
            onClick={onSelectAll}
            className="px-2.5 py-1.5 rounded-md text-[11px] bg-surface text-text-sub hover:bg-surface-hi hover:text-text"
          >
            All
          </button>
          <button
            type="button"
            onClick={onClearAll}
            className="px-2.5 py-1.5 rounded-md text-[11px] bg-surface text-text-sub hover:bg-surface-hi hover:text-text"
          >
            None
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-3">
          <div className="flex flex-col gap-0.5">
            {nodes.map((node) => (
              <NodeRow
                key={`${node.path}-${node.entity_name ?? ""}`}
                node={node}
                selectedPaths={selectedPaths}
                onToggle={onToggle}
              />
            ))}
          </div>
        </div>

        <div className="p-4 border-t border-border flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-3 py-2 rounded-md text-xs bg-surface text-text-sub hover:bg-surface-hi hover:text-text"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy || selectedCount === 0}
            className={`px-3 py-2 rounded-md text-xs font-medium ${
              busy || selectedCount === 0
                ? "bg-surface text-text-faint cursor-not-allowed"
                : "bg-accent text-on-accent hover:brightness-110"
            }`}
          >
            Export selected
          </button>
        </div>
      </div>
    </div>
  );
}
