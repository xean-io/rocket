import { MetaLabel } from "@/components/MetaLabel";

/** Page title with a quiet metadata line (same rhythm as the project header). */
export function SectionHeader({ title, detail }: { title: string; detail?: React.ReactNode }) {
  return (
    <div className="space-y-1 px-6 pt-5 pb-3.5">
      <h2 className="truncate text-[1.7rem] leading-tight font-normal tracking-tight text-ink">{title}</h2>
      {detail && <MetaLabel className="block">{detail}</MetaLabel>}
    </div>
  );
}
