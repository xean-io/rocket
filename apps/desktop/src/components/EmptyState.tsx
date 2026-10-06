import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/** Centered empty / unavailable state that fills the space below a header. */
export function EmptyState({
  icon: Icon,
  title,
  description,
  children,
  className,
}: {
  icon: LucideIcon;
  title: string;
  description?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-8 text-center", className)}>
      <span className="flex size-10 items-center justify-center rounded-full border border-border bg-surface text-secondary-ink">
        <Icon className="size-4.5" aria-hidden />
      </span>
      <div className="space-y-1">
        <h2 className="text-base font-medium text-ink">{title}</h2>
        {description && <p className="mx-auto max-w-[27rem] text-sm text-secondary-ink">{description}</p>}
      </div>
      {children}
    </div>
  );
}
