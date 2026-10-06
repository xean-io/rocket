import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ArrowRightCircle, Copy, Network, TriangleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import { EmptyState } from "@/components/EmptyState";
import { MetaLabel } from "@/components/MetaLabel";
import { SectionHeader } from "@/components/SectionHeader";
import { projectPath } from "@/components/shell/AppSidebar";
import { TopBar } from "@/components/shell/TopBar";
import { StatusGlyph } from "@/components/StatusGlyph";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "@/components/ui/context-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { Conflict, PortInfo, Run } from "@/lib/bindings";
import { formatOwner } from "@/lib/format";
import { useNav } from "@/lib/nav";
import { blockedConflicts, portUrl, portsSummary, remapFor, remapLabel } from "@/lib/ports";
import { useConflicts, usePorts, useProjectNames, useRuns } from "@/lib/queries";
import { statusStyle } from "@/lib/status";
import { cn } from "@/lib/utils";

async function copyUrl(port: number) {
  const url = portUrl(port);
  try {
    await writeText(url);
    toast.success("URL copied", { description: url });
  } catch {
    try {
      await navigator.clipboard.writeText(url);
      toast.success("URL copied", { description: url });
    } catch (e) {
      toast.error("Could not copy the URL", { description: String(e) });
    }
  }
}

function PortRow({
  port,
  remap,
  state,
  selected,
  onSelect,
  onShow,
}: {
  port: PortInfo;
  remap?: Conflict;
  state: Run["state"];
  selected: boolean;
  onSelect: () => void;
  onShow: () => void;
}) {
  const style = statusStyle(state);
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <TableRow
            data-selected={selected}
            aria-selected={selected}
            data-port={port.port}
            onClick={onSelect}
            onContextMenu={onSelect}
            onDoubleClick={onShow}
            className={cn(
              "h-9 cursor-default border-b-[length:var(--hairline-width)]",
              "hover:bg-primary/8 data-[selected=true]:bg-primary/20 data-[selected=true]:shadow-[inset_2px_0_0_var(--primary)]",
            )}
          />
        }
      >
        <TableCell className="py-1">
          <span className="selectable flex items-center gap-2">
            <span className={cn("font-mono text-[0.9rem]", remap ? "text-status-warn" : "text-ink")}>{port.port}</span>
            {remap && (
              <MetaLabel className="text-status-warn" title={remap.detail}>
                {remapLabel(remap)}
              </MetaLabel>
            )}
          </span>
        </TableCell>
        <TableCell className="py-1">
          <span className="truncate">
            {port.project}/{port.service}
          </span>
        </TableCell>
        <TableCell className="py-1">
          <MetaLabel>{port.port_name}</MetaLabel>
        </TableCell>
        <TableCell className="py-1">
          <span className="flex items-center gap-2">
            <StatusGlyph style={style} />
            {style.label}
          </span>
        </TableCell>
        <TableCell className="py-1">
          <MetaLabel>{formatOwner(port.owner)}</MetaLabel>
        </TableCell>
        <TableCell className="py-1 font-mono text-[0.82rem] text-secondary-ink">{port.pid ?? "—"}</TableCell>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-44">
        <ContextMenuItem onClick={onShow}>
          <ArrowRightCircle /> Show Service
        </ContextMenuItem>
        <ContextMenuItem onClick={() => void copyUrl(port.port)}>
          <Copy /> Copy URL
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

const heads = ["Port", "Service", "Name", "State", "Owner", "PID"];

/** Global port map: remapped leases and blocked ports are highlighted. */
export function PortsPage() {
  const navigate = useNavigate();
  const ports = usePorts();
  const runs = useRuns();
  const { names } = useProjectNames();
  const conflicts = useConflicts(names);
  const [selected, setSelected] = useState<number | null>(null);

  const list = useMemo(() => [...(ports.data?.ports ?? [])].sort((a, b) => a.port - b.port), [ports.data]);
  const runState = useMemo(() => {
    const map = new Map<string, Run["state"]>();
    for (const r of runs.data?.services ?? []) map.set(`${r.project}/${r.service}`, r.state);
    return map;
  }, [runs.data]);
  const blocked = blockedConflicts(conflicts);

  const show = (p: PortInfo) => {
    useNav.getState().selectService({ project: p.project, service: p.service });
    useNav.getState().setInspectorOpen(true);
    void navigate(projectPath(p.project));
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TopBar title="Ports" />
      <SectionHeader title="Ports" detail={portsSummary(list, conflicts)} />
      {blocked.length > 0 && (
        <ul aria-label="Blocked ports" className="hairline-t space-y-1.5 px-6 py-2.5 text-sm">
          {blocked.map((c) => (
            <li key={`${c.project}/${c.service}/${c.port_name}`} className="flex items-start gap-2">
              <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-status-warn" aria-hidden />
              <span>
                <span className="font-medium text-ink">
                  {c.project}/{c.service}
                </span>{" "}
                <span className="text-secondary-ink">{c.detail || `port ${c.port} busy`}</span>
              </span>
            </li>
          ))}
        </ul>
      )}
      <div className="hairline-t flex min-h-0 flex-1 flex-col">
        {!ports.isPending && list.length === 0 ? (
          <EmptyState icon={Network} title="No leased ports" description="Ports appear here while services hold them." />
        ) : (
          <div className="min-h-0 flex-1 overflow-auto bg-surface/60">
            <Table className="min-w-[40rem]">
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  {heads.map((h) => (
                    <TableHead key={h} className="meta h-8 font-normal">
                      {h}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {ports.isPending
                  ? Array.from({ length: 3 }, (_, i) => (
                      <TableRow key={i} className="h-9 hover:bg-transparent">
                        <TableCell colSpan={6}>
                          <Skeleton className="h-4 w-full max-w-[30rem]" />
                        </TableCell>
                      </TableRow>
                    ))
                  : list.map((p) => (
                      <PortRow
                        key={p.port}
                        port={p}
                        remap={remapFor(p, conflicts)}
                        state={runState.get(`${p.project}/${p.service}`) ?? p.state ?? "stopped"}
                        selected={selected === p.port}
                        onSelect={() => setSelected(p.port)}
                        onShow={() => show(p)}
                      />
                    ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>
    </div>
  );
}
