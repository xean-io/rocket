import { cn } from "@/lib/utils";

/** Quiet monospaced, uppercase, tracked label (kinds, owners, key names). */
export function MetaLabel({ className, ...props }: React.ComponentProps<"span">) {
  return <span className={cn("meta truncate", className)} {...props} />;
}
