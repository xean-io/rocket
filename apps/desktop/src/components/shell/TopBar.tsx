import { ConnectionBadge } from "@/components/shell/ConnectionViews";
import { cn } from "@/lib/utils";

/**
 * Title strip of the main pane. It doubles as the window drag region, so
 * interactive children sit on top of empty drag surface.
 */
export function TopBar({
  title,
  children,
  className,
}: {
  title?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <header
      data-tauri-drag-region
      className={cn("hairline-b flex h-11 shrink-0 items-center gap-2 px-4", className)}
    >
      {title && (
        <h1 data-tauri-drag-region className="mr-1 text-sm font-medium text-ink">
          {title}
        </h1>
      )}
      <div data-tauri-drag-region className="min-w-3 flex-1 self-stretch" />
      {children}
      <ConnectionBadge />
    </header>
  );
}
