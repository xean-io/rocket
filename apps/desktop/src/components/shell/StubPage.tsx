import type { LucideIcon } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { TopBar } from "@/components/shell/TopBar";

/** Placeholder for sections that land in a later task; still a real empty state. */
export function StubPage({
  title,
  icon,
  description,
}: {
  title: string;
  icon: LucideIcon;
  description: string;
}) {
  return (
    <div className="flex h-full flex-col">
      <TopBar title={title} />
      <EmptyState icon={icon} title={title} description={description} />
    </div>
  );
}
