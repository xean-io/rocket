import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Check, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { StatusCapsules } from "@/components/project/StatusCapsules";
import { Button } from "@/components/ui/button";
import type { Conflict, Run } from "@/lib/bindings";

async function copyText(text: string) {
  try {
    await writeText(text);
  } catch {
    await navigator.clipboard?.writeText(text);
  }
}

/** Project title, path (truncated, copyable) and status capsules. */
export function ProjectHeader({
  name,
  path,
  runs,
  conflicts,
  busy,
}: {
  name: string;
  path?: string;
  runs: readonly Run[];
  conflicts: readonly Conflict[];
  busy: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const copy = () => {
    if (!path) return;
    void copyText(path).then(() => {
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1400);
    });
  };

  return (
    <div className="@container flex items-end gap-4 px-6 pt-5 pb-3.5">
      <div className="min-w-0 flex-1 space-y-1">
        <h2 className="truncate text-[1.7rem] leading-tight font-normal tracking-tight text-ink">{name}</h2>
        {path && (
          <div className="flex min-w-0 items-center gap-1">
            <span className="selectable truncate font-mono text-xs text-secondary-ink" title={path}>
              {path}
            </span>
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={copy}
              aria-label={copied ? "Path copied" : "Copy path"}
              title={copied ? "Copied" : "Copy path"}
            >
              {copied ? <Check className="text-status-ok" /> : <Copy />}
            </Button>
          </div>
        )}
      </div>
      <StatusCapsules runs={runs} conflicts={conflicts} busy={busy} />
    </div>
  );
}
