import { ArrowDownToLine } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const NEAR_BOTTOM_PX = 24;

/**
 * Monospaced log tail. Sticks to the bottom as lines arrive, unless the user
 * scrolled up; "Jump to latest" resumes following.
 */
export function LogTail({ lines, emptyText = "No output yet" }: { lines: readonly string[]; emptyText?: string }) {
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [detached, setDetached] = useState(false);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= NEAR_BOTTOM_PX;
    stick.current = atBottom;
    setDetached(!atBottom);
  };

  const jump = () => {
    const el = scroller.current;
    if (!el) return;
    stick.current = true;
    el.scrollTop = el.scrollHeight;
    setDetached(false);
  };

  return (
    <div className="relative min-h-0 flex-1 bg-surface">
      <div
        ref={scroller}
        onScroll={onScroll}
        role="log"
        aria-label={`Log output, ${lines.length} lines`}
        className="selectable h-full overflow-auto p-3 font-mono text-[0.72rem] leading-[1.45]"
      >
        {lines.map((line, i) => (
          <div
            key={i}
            className={cn(
              "break-all whitespace-pre-wrap",
              line.startsWith("=== rocket:") ? "text-violet" : "text-ink/90",
            )}
          >
            {line || " "}
          </div>
        ))}
      </div>
      {lines.length === 0 && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-secondary-ink">
          {emptyText}
        </div>
      )}
      {detached && (
        <Button size="xs" variant="secondary" className="absolute right-3 bottom-3 shadow-sm" onClick={jump}>
          <ArrowDownToLine /> Jump to latest
        </Button>
      )}
    </div>
  );
}
