import { ChevronDown, ListChecks, Layers, PanelRight, Play, RefreshCw, Send, Square, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { StatusGlyph } from "@/components/StatusGlyph";
import type { Run } from "@/lib/bindings";
import { statusStyle } from "@/lib/status";

const DEFAULT_ENV = "__default__";

export interface ProjectToolbarProps {
  /** The selected service, if any: Up/Restart/Down then act on it alone. */
  selected: Run | null;
  onClearSelection: () => void;
  envs: string[];
  defaultEnv?: string;
  /** Explicit env choice; undefined means the project default. */
  env?: string;
  onEnvChange: (env: string | undefined) => void;
  pipelines: string[];
  deployEnvs: string[];
  busy: boolean;
  online: boolean;
  inspectorOpen: boolean;
  onUp: () => void;
  onRestart: () => void;
  onDown: () => void;
  onRunPipeline: (name: string) => void;
  onDeploy: (env: string) => void;
  onToggleInspector: () => void;
}

/** Title-strip controls of a project: env, lifecycle, pipelines, deploy, logs. */
export function ProjectToolbar(p: ProjectToolbarProps) {
  const target = p.selected ? p.selected.service : "every service";
  const envItems = [
    { value: DEFAULT_ENV, label: p.defaultEnv ? `Default (${p.defaultEnv})` : "Default" },
    ...p.envs.map((e) => ({ value: e, label: e })),
  ];
  return (
    <>
      {p.selected && (
        <button
          type="button"
          onClick={p.onClearSelection}
          title="Clear selection to act on the whole project"
          aria-label={`Selected ${p.selected.service}. Clear selection`}
          className="flex h-7 max-w-44 items-center gap-1.5 rounded-full border border-primary/50 bg-primary/15 pr-1.5 pl-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          <StatusGlyph style={statusStyle(p.selected.state, p.selected.health)} />
          <span className="truncate">{p.selected.service}</span>
          <X className="size-3 shrink-0 text-secondary-ink" />
        </button>
      )}

      <Select
        value={p.env ?? DEFAULT_ENV}
        onValueChange={(v) => p.onEnvChange(v === DEFAULT_ENV ? undefined : (v as string))}
        items={envItems}
        disabled={p.envs.length === 0}
      >
        <SelectTrigger size="sm" aria-label="Environment" title="Environment used by Up and Restart" className="min-w-28">
          <Layers className="text-secondary-ink" />
          <SelectValue />
        </SelectTrigger>
        <SelectContent alignItemWithTrigger={false} className="min-w-40">
          {envItems.map((i) => (
            <SelectItem key={i.value} value={i.value}>
              {i.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      <div className="flex items-center gap-1">
        <Button size="sm" disabled={!p.online || p.busy} onClick={p.onUp} title={`Start ${target} (⇧⌘U)`}>
          <Play /> Up
        </Button>
        <Button size="sm" variant="outline" disabled={!p.online || p.busy} onClick={p.onRestart} title={`Restart ${target} (⌘R)`}>
          <RefreshCw /> Restart
        </Button>
        <Button size="sm" variant="outline" disabled={!p.online || p.busy} onClick={p.onDown} title={`Stop ${target} (⌘.)`}>
          <Square /> Down
        </Button>
      </div>

      <div className="flex items-center gap-1">
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                size="sm"
                variant="outline"
                disabled={!p.online || p.pipelines.length === 0}
                title={
                  p.pipelines.length === 0
                    ? "No pipelines declared in rocket.yaml"
                    : "Run a configured pipeline in the selected environment"
                }
              />
            }
          >
            <ListChecks /> Run pipeline <ChevronDown className="opacity-60" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-44">
            {p.pipelines.map((name) => (
              <DropdownMenuItem key={name} onClick={() => p.onRunPipeline(name)}>
                {name}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                size="sm"
                variant="outline"
                disabled={!p.online || p.deployEnvs.length === 0}
                title={
                  p.deployEnvs.length === 0
                    ? "No deploy environments declared in rocket.yaml"
                    : "Deploy to a configured environment after confirmation"
                }
              />
            }
          >
            <Send /> Deploy… <ChevronDown className="opacity-60" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-44">
            {p.deployEnvs.map((env) => (
              <DropdownMenuItem key={env} onClick={() => p.onDeploy(env)}>
                {env}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <Button
        size="icon-sm"
        variant={p.inspectorOpen ? "secondary" : "ghost"}
        aria-pressed={p.inspectorOpen}
        aria-label="Toggle logs inspector"
        title="Show service details and logs (⌘L)"
        onClick={p.onToggleInspector}
      >
        <PanelRight />
      </Button>
    </>
  );
}
