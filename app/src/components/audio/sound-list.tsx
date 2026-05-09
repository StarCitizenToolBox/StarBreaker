import { Download } from "lucide-react";
import { audioExportInfo, audioExportMedia } from "../../lib/commands";
import { useAudioStore } from "../../stores/audio-store";

export function SoundList() {
  const sounds = useAudioStore((s) => s.sounds);
  const currentSound = useAudioStore((s) => s.currentSound);
  const playSound = useAudioStore((s) => s.playSound);
  const selectedTrigger = useAudioStore((s) => s.selectedTrigger);

  const handleExport = async (
    mediaId: number,
    sourceType: string,
    bankName: string,
  ) => {
    let extension = "wem";
    try {
      const info = await audioExportInfo(mediaId, sourceType, bankName);
      extension = info.extension;
    } catch (err) {
      useAudioStore.setState({ error: String(err) });
      return;
    }

    const { save } = await import("@tauri-apps/plugin-dialog");
    const outputPath = await save({
      title: `Export ${mediaId}`,
      defaultPath: `${mediaId}.${extension}`,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!outputPath) return;

    try {
      await audioExportMedia(mediaId, sourceType, bankName, outputPath);
    } catch (err) {
      useAudioStore.setState({ error: String(err) });
    }
  };

  return (
    <div className="flex-1 min-w-[200px] flex flex-col overflow-hidden">
      <div className="px-3 py-1.5 text-xs font-medium text-text-dim border-b border-border bg-bg-alt">
        Sounds {sounds.length > 0 && `(${sounds.length})`}
      </div>
      <div className="flex-1 overflow-y-auto">
        {sounds.map((sound, index) => {
          const isActive = currentSound?.media_id === sound.media_id;
          return (
            <div
              key={`${sound.media_id}-${index}`}
              className={`group flex items-center gap-2 px-3 py-1.5 text-sm ${
                isActive
                  ? "bg-primary/15 text-text"
                  : "text-text-sub hover:bg-surface/50"
              }`}
            >
              <button
                type="button"
                onClick={() => playSound(sound)}
                className="shrink-0 w-6 h-6 flex items-center justify-center rounded bg-surface hover:bg-surface-hi transition-colors text-xs"
                title={`Play ${sound.media_id}`}
              >
                {isActive ? "||" : "▶"}
              </button>
              <span className="font-mono text-xs">{sound.media_id}</span>
              <span
                className={`text-xs px-1.5 py-0.5 rounded ${
                  sound.source_type === "Embedded"
                    ? "bg-success/15 text-success"
                    : "bg-warning/15 text-warning"
                }`}
              >
                {sound.source_type}
              </span>
              <button
                type="button"
                onClick={() =>
                  handleExport(sound.media_id, sound.source_type, sound.bank_name)
                }
                title={`Export ${sound.media_id}`}
                className="ml-auto hidden group-hover:flex items-center justify-center w-5 h-5 rounded
                           text-text-dim hover:text-text hover:bg-surface-hi transition-colors"
              >
                <Download size={12} />
              </button>
            </div>
          );
        })}
        {sounds.length === 0 && (
          <div className="px-3 py-4 text-xs text-text-faint text-center">
            {selectedTrigger ? "No sounds resolved" : "Select a trigger to see sounds"}
          </div>
        )}
      </div>
    </div>
  );
}
