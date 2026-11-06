"use client";

import * as Dialog from "@radix-ui/react-dialog";
import {
  Activity, AlertTriangle, Archive, Box, CheckCircle2, ChevronRight, CircleDot,
  Clock3, Code2, Copy, Download, FileArchive, FileCode2, FileUp, Gauge, HardDrive,
  Library, Menu, MonitorCog, Network, PackagePlus, Pause, Play, RefreshCw, Search,
  Server, ShieldCheck, SlidersHorizontal, TerminalSquare, Trash2, Upload, Users, X,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { dashboardTransport } from "@/lib/transports";
import type {
  BootImage, ClientSnapshot, DownloadImageInput, ImagePreset, Protocol, PxeEvent,
  StatusSnapshot,
} from "@/lib/types";
import { cn } from "@/lib/utils";

type View = "overview" | "clients" | "library" | "scripts" | "activity";
type ToastTone = "success" | "error" | "info";
type Toast = { message: string; tone: ToastTone } | null;

const navigation: Array<{ id: View; label: string; icon: typeof Gauge }> = [
  { id: "overview", label: "Overview", icon: Gauge },
  { id: "clients", label: "Boot clients", icon: Users },
  { id: "library", label: "Boot library", icon: Library },
  { id: "scripts", label: "iPXE scripts", icon: Code2 },
  { id: "activity", label: "Activity", icon: Activity },
];

function formatBytes(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatUptime(seconds: number) {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}

function stageTone(stage: string) {
  const value = stage.toLowerCase();
  if (value.includes("booted")) return "success";
  if (value.includes("tftp") || value.includes("http")) return "active";
  if (value.includes("discover")) return "warning";
  return "neutral";
}

function protocolClass(protocol: Protocol) {
  return `protocol protocol-${protocol.toLowerCase()}`;
}

export function DashboardApp() {
  const [view, setView] = useState<View>("overview");
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [status, setStatus] = useState<StatusSnapshot | null>(null);
  const [images, setImages] = useState<BootImage[]>([]);
  const [presets, setPresets] = useState<ImagePreset[]>([]);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [lastSuccess, setLastSuccess] = useState<Date | null>(null);
  const [toast, setToast] = useState<Toast>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [actionPending, setActionPending] = useState<string | null>(null);
  const toastTimer = useRef<number | null>(null);

  const notify = useCallback((message: string, tone: ToastTone = "info") => {
    setToast({ message, tone });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 4200);
  }, []);

  const loadStatusNow = useCallback(async () => {
    try {
      const next = await dashboardTransport.getStatus();
      setStatus(next);
      setConnected(true);
      setLastSuccess(new Date());
      setLoading(false);
      return true;
    } catch {
      setConnected(false);
      setLoading(false);
      return false;
    }
  }, []);

  const loadImagesNow = useCallback(async () => {
    try {
      setImages(await dashboardTransport.getImages());
    } catch {
      // Status connection state owns the global offline indication.
    }
  }, []);

  useEffect(() => {
    const initial = window.location.hash.replace("#", "") as View;
    if (navigation.some((item) => item.id === initial)) setView(initial);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    let delay = 1500;
    const tick = async () => {
      if (document.visibilityState === "hidden") {
        timer = window.setTimeout(tick, 2500);
        return;
      }
      const ok = await loadStatusNow();
      delay = ok ? 1500 : Math.min(delay * 2, 15000);
      if (!cancelled) timer = window.setTimeout(tick, delay);
    };
    void tick();
    const onVisible = () => document.visibilityState === "visible" && void loadStatusNow();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [loadStatusNow]);

  useEffect(() => {
    void loadImagesNow();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void loadImagesNow();
    }, 8000);
    return () => window.clearInterval(timer);
  }, [loadImagesNow]);

  const selectView = (next: View) => {
    setView(next);
    setSidebarOpen(false);
    window.history.replaceState(null, "", `#${next}`);
  };

  const execute = async (key: string, task: () => Promise<void>, success: string) => {
    setActionPending(key);
    try {
      await task();
      notify(success, "success");
      await Promise.all([loadStatusNow(), loadImagesNow()]);
    } catch (error) {
      notify(error instanceof Error ? error.message : "The action failed.", "error");
    } finally {
      setActionPending(null);
    }
  };

  const disconnectClient = (client: ClientSnapshot) => {
    if (!window.confirm(`Remove ${client.mac} from the active client list?`)) return;
    void execute(`client-${client.mac}`, () => dashboardTransport.disconnectClient(client.mac), "Client removed.");
  };

  const clearClients = () => {
    if (!window.confirm("Remove every client from the active session list?")) return;
    void execute("clear-clients", () => dashboardTransport.clearClients(), "Active clients cleared.");
  };

  const activeClients = status?.clients ?? [];
  const metrics = status?.metrics;
  const isDemo = dashboardTransport.mode === "demo";

  return (
    <div className="app-shell">
      <Sidebar view={view} open={sidebarOpen} onSelect={selectView} onClose={() => setSidebarOpen(false)} />

      <main className="main-shell">
        <header className="topbar">
          <div className="topbar-leading">
            <Button className="mobile-menu" size="icon" variant="ghost" aria-label="Open navigation" onClick={() => setSidebarOpen(true)}>
              <Menu size={20} />
            </Button>
            <div>
              <p className="eyebrow">Network operations</p>
              <h1>{navigation.find((item) => item.id === view)?.label}</h1>
            </div>
          </div>
          <div className="topbar-actions">
            {isDemo && <span className="demo-badge"><CircleDot size={13} /> Vercel demo</span>}
            <div className={cn("connection-pill", connected ? "connected" : "disconnected")}>
              <span className="connection-dot" />
              <div><strong>{connected ? "Server online" : "Connection lost"}</strong><small>{lastSuccess ? `Updated ${lastSuccess.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}` : "Waiting for telemetry"}</small></div>
            </div>
            <Button size="icon" variant="secondary" aria-label="Refresh dashboard" onClick={() => { void loadStatusNow(); void loadImagesNow(); }}>
              <RefreshCw size={17} />
            </Button>
          </div>
        </header>

        {isDemo && (
          <div className="demo-banner">
            <ShieldCheck size={17} />
            <span><strong>Preview environment.</strong> Data is simulated and server-changing actions are disabled.</span>
          </div>
        )}

        {!connected && status && (
          <div className="offline-banner"><AlertTriangle size={17} /> Showing the last successful snapshot while PXE reconnects.</div>
        )}

        <div className="page-content">
          {loading && !status ? <LoadingState /> : (
            <>
              {view === "overview" && <Overview status={status} images={images} onNavigate={selectView} />}
              {view === "clients" && <ClientsView clients={activeClients} disabled={isDemo} pending={actionPending} onDisconnect={disconnectClient} onClear={clearClients} />}
              {view === "library" && <LibraryView images={images} onAdd={() => setAddOpen(true)} disabled={isDemo} />}
              {view === "scripts" && <ScriptsView notify={notify} />}
              {view === "activity" && <ActivityView events={status?.events ?? []} />}
            </>
          )}
        </div>
      </main>

      <AddImageDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        disabled={isDemo}
        presets={presets}
        loadPresets={async () => setPresets(await dashboardTransport.getPresets())}
        pending={actionPending}
        onPreset={(preset) => void execute(`preset-${preset.id}`, () => dashboardTransport.downloadPreset(preset.id), `${preset.name} download scheduled.`)}
        onUrl={(input) => void execute("url-download", () => dashboardTransport.downloadImage(input), "Image download scheduled.")}
        onUploaded={() => { notify("Image uploaded.", "success"); void loadImagesNow(); setAddOpen(false); }}
        notify={notify}
      />

      {toast && (
        <div className={cn("toast", `toast-${toast.tone}`)} role="status">
          {toast.tone === "success" ? <CheckCircle2 size={18} /> : toast.tone === "error" ? <XCircle size={18} /> : <CircleDot size={18} />}
          {toast.message}
          <button onClick={() => setToast(null)} aria-label="Dismiss notification"><X size={15} /></button>
        </div>
      )}
    </div>
  );
}

function Sidebar({ view, open, onSelect, onClose }: { view: View; open: boolean; onSelect: (view: View) => void; onClose: () => void }) {
  return (
    <>
      {open && <button className="sidebar-backdrop" onClick={onClose} aria-label="Close navigation" />}
      <aside className={cn("sidebar", open && "sidebar-open")}>
        <div className="brand-lockup">
          <div className="brand-mark"><Network size={22} /></div>
          <div><strong>PXE Control</strong><span>Netboot appliance</span></div>
          <Button className="sidebar-close" size="icon" variant="ghost" aria-label="Close navigation" onClick={onClose}><X size={18} /></Button>
        </div>
        <nav aria-label="Primary navigation">
          <p className="nav-label">Workspace</p>
          {navigation.map((item) => {
            const Icon = item.icon;
            return (
              <button key={item.id} className={cn("nav-item", item.id === view && "active")} onClick={() => onSelect(item.id)} aria-current={item.id === view ? "page" : undefined}>
                <Icon size={18} /><span>{item.label}</span><ChevronRight className="nav-chevron" size={15} />
              </button>
            );
          })}
        </nav>
        <div className="sidebar-card">
          <div className="sidebar-card-icon"><TerminalSquare size={17} /></div>
          <div><strong>PXE-RS 0.1</strong><span>DHCP · TFTP · HTTP</span></div>
        </div>
      </aside>
    </>
  );
}

function Overview({ status, images, onNavigate }: { status: StatusSnapshot | null; images: BootImage[]; onNavigate: (view: View) => void }) {
  if (!status) return <EmptyState icon={Server} title="No telemetry yet" body="The console is waiting for the PXE service." />;
  const { metrics, clients, events } = status;
  const cards = [
    { label: "Active clients", value: clients.length.toString(), detail: `${clients.filter((c) => c.last_seen_secs_ago < 30).length} seen in 30 seconds`, icon: Users, tone: "blue" },
    { label: "DHCP exchange", value: `${metrics.dhcp_out}/${metrics.dhcp_in}`, detail: "offers per discovery", icon: Network, tone: "cyan" },
    { label: "TFTP payload", value: formatBytes(metrics.tftp_bytes), detail: `${metrics.tftp_files} files served`, icon: HardDrive, tone: "amber" },
    { label: "HTTP requests", value: metrics.http_requests.toLocaleString(), detail: "kernels, initrds and scripts", icon: Download, tone: "violet" },
    { label: "Protocol health", value: metrics.errors === 0 ? "Healthy" : `${metrics.errors} errors`, detail: metrics.errors === 0 ? "no recorded failures" : "review activity stream", icon: metrics.errors === 0 ? CheckCircle2 : AlertTriangle, tone: metrics.errors === 0 ? "green" : "red" },
  ];
  return (
    <div className="stack-lg">
      <section className="hero-panel">
        <div>
          <span className="hero-kicker"><span className="live-dot" /> PXE engine active</span>
          <h2>{status.server_ip}:{status.http_port}</h2>
          <p>ProxyDHCP, TFTP and HTTP services are accepting clients on the local network.</p>
        </div>
        <div className="hero-meta"><Clock3 size={18} /><span>Uptime<strong>{formatUptime(status.uptime_secs)}</strong></span></div>
      </section>

      <section className="metric-grid" aria-label="Server metrics">
        {cards.map((card) => {
          const Icon = card.icon;
          return <article className="metric-card" key={card.label}><div className={cn("metric-icon", `tone-${card.tone}`)}><Icon size={19} /></div><span>{card.label}</span><strong>{card.value}</strong><small>{card.detail}</small></article>;
        })}
      </section>

      <div className="overview-grid">
        <section className="panel">
          <PanelHeader title="Boot clients" eyebrow={`${clients.length} sessions`} action={<Button size="sm" variant="ghost" onClick={() => onNavigate("clients")}>View all <ChevronRight size={15} /></Button>} />
          <div className="compact-list">
            {clients.slice(0, 5).map((client) => <ClientCompact key={client.mac} client={client} />)}
            {clients.length === 0 && <InlineEmpty text="Listening for PXE clients…" />}
          </div>
        </section>
        <section className="panel">
          <PanelHeader title="Recent activity" eyebrow={`${events.length} events`} action={<Button size="sm" variant="ghost" onClick={() => onNavigate("activity")}>Open stream <ChevronRight size={15} /></Button>} />
          <div className="event-list compact-events">
            {events.slice(-6).reverse().map((event, index) => <EventRow key={`${event.timestamp}-${index}`} event={event} />)}
            {events.length === 0 && <InlineEmpty text="No events recorded yet." />}
          </div>
        </section>
      </div>

      <section className="panel library-summary">
        <PanelHeader title="Boot library" eyebrow={`${images.length} files across ${new Set(images.map((image) => image.category)).size} categories`} action={<Button size="sm" variant="ghost" onClick={() => onNavigate("library")}>Manage library <ChevronRight size={15} /></Button>} />
        <div className="category-row">
          {Array.from(new Set(images.map((image) => image.category))).slice(0, 6).map((category) => <span className="category-chip" key={category}><Archive size={14} /> {category}</span>)}
          {images.length === 0 && <InlineEmpty text="No boot images found in httpboot/." />}
        </div>
      </section>
    </div>
  );
}

function ClientsView({ clients, disabled, pending, onDisconnect, onClear }: { clients: ClientSnapshot[]; disabled: boolean; pending: string | null; onDisconnect: (client: ClientSnapshot) => void; onClear: () => void }) {
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<"recent" | "mac" | "stage">("recent");
  const visible = useMemo(() => clients.filter((client) => `${client.mac} ${client.ip ?? ""} ${client.arch} ${client.stage_text}`.toLowerCase().includes(query.toLowerCase())).sort((a, b) => sort === "recent" ? a.last_seen_secs_ago - b.last_seen_secs_ago : sort === "mac" ? a.mac.localeCompare(b.mac) : a.stage_text.localeCompare(b.stage_text)), [clients, query, sort]);
  return (
    <section className="panel full-panel">
      <div className="toolbar">
        <div><p className="eyebrow">Live session inventory</p><h2>Connected boot clients</h2><p>{clients.length} clients tracked by DHCP, TFTP, and HTTP activity.</p></div>
        <Button variant="danger" disabled={disabled || clients.length === 0 || pending === "clear-clients"} onClick={onClear}><Trash2 size={16} /> Clear all</Button>
      </div>
      <div className="filterbar">
        <label className="search-field"><Search size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search MAC, IP, architecture or stage" /></label>
        <label className="select-field"><SlidersHorizontal size={15} /><select value={sort} onChange={(event) => setSort(event.target.value as typeof sort)}><option value="recent">Most recent</option><option value="mac">MAC address</option><option value="stage">Boot stage</option></select></label>
      </div>
      {visible.length === 0 ? <EmptyState icon={Users} title={query ? "No matching clients" : "Waiting for clients"} body={query ? "Try a different search term." : "Clients appear here as soon as they begin PXE discovery."} /> : (
        <div className="table-wrap"><table><thead><tr><th>Client</th><th>Assigned IP</th><th>Architecture</th><th>Boot stage</th><th>Last seen</th><th><span className="sr-only">Actions</span></th></tr></thead><tbody>{visible.map((client) => <tr key={client.mac}><td><strong className="mono">{client.mac}</strong></td><td className="mono">{client.ip ?? <span className="muted">Acquiring…</span>}</td><td>{client.arch}</td><td><span className={cn("stage-badge", `stage-${stageTone(client.stage_text)}`)}>{client.stage_text}</span></td><td>{client.last_seen_secs_ago}s ago</td><td className="row-action"><Button size="sm" variant="ghost" disabled={disabled || pending === `client-${client.mac}`} onClick={() => onDisconnect(client)}><X size={15} /> Remove</Button></td></tr>)}</tbody></table></div>
      )}
    </section>
  );
}

function LibraryView({ images, onAdd, disabled }: { images: BootImage[]; onAdd: () => void; disabled: boolean }) {
  const [query, setQuery] = useState("");
  const groups = useMemo(() => {
    const filtered = images.filter((image) => `${image.name} ${image.rel_path} ${image.category}`.toLowerCase().includes(query.toLowerCase()));
    return Object.entries(Object.groupBy(filtered, (image) => image.category));
  }, [images, query]);
  return (
    <div className="stack-lg">
      <section className="toolbar standalone-toolbar">
        <div><p className="eyebrow">httpboot/ repository</p><h2>Configured boot images</h2><p>Manage kernels, initrds, disk images, and chainloaders served by PXE.</p></div>
        <Button variant="primary" disabled={disabled} onClick={onAdd}><PackagePlus size={17} /> Add boot image</Button>
      </section>
      <div className="filterbar panel-filter"><label className="search-field"><Search size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search the boot library" /></label><span className="result-count">{images.length} files · {formatBytes(images.reduce((sum, item) => sum + item.size_bytes, 0))}</span></div>
      {groups.length === 0 ? <EmptyState icon={Library} title={query ? "No matching images" : "Boot library is empty"} body={query ? "Try a different search term." : "Add an official preset, remote image, or local file."} action={!disabled ? <Button variant="primary" onClick={onAdd}><PackagePlus size={16} /> Add first image</Button> : undefined} /> : (
        <div className="library-groups">{groups.map(([category, entries]) => <section className="panel image-group" key={category}><PanelHeader title={category} eyebrow={`${entries?.length ?? 0} files`} /><div className="image-grid">{entries?.map((image) => <article className="image-card" key={image.rel_path}><div className="file-icon">{image.is_iso ? <FileArchive /> : image.is_kernel ? <MonitorCog /> : image.is_initrd ? <Box /> : <FileCode2 />}</div><div className="image-details"><strong>{image.name}</strong><span className="mono">{image.rel_path}</span></div><span className="file-size">{image.size_formatted}</span></article>)}</div></section>)}</div>
      )}
    </div>
  );
}

function ScriptsView({ notify }: { notify: (message: string, tone?: ToastTone) => void }) {
  const [tab, setTab] = useState<"boot" | "autoexec">("boot");
  const [scripts, setScripts] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (scripts[tab]) return;
    setLoading(true);
    dashboardTransport.getScript(tab).then((value) => setScripts((current) => ({ ...current, [tab]: value }))).catch((error) => notify(error instanceof Error ? error.message : "Script could not be loaded.", "error")).finally(() => setLoading(false));
  }, [tab, scripts, notify]);
  const content = scripts[tab] ?? "";
  const copy = async () => { await navigator.clipboard.writeText(content); notify("Script copied to clipboard.", "success"); };
  const download = () => { const anchor = document.createElement("a"); anchor.href = URL.createObjectURL(new Blob([content], { type: "text/plain" })); anchor.download = `${tab}.ipxe`; anchor.click(); URL.revokeObjectURL(anchor.href); };
  return (
    <section className="panel script-panel">
      <div className="toolbar"><div><p className="eyebrow">Boot orchestration</p><h2>iPXE scripts</h2><p>Inspect the generated menu and first-stage chainloader served to clients.</p></div><div className="toolbar-actions"><Button onClick={() => void copy()} disabled={!content}><Copy size={16} /> Copy</Button><Button onClick={download} disabled={!content}><Download size={16} /> Download</Button></div></div>
      <div className="script-tabs" role="tablist"><button role="tab" aria-selected={tab === "boot"} className={tab === "boot" ? "active" : ""} onClick={() => setTab("boot")}>boot.ipxe<span>Menu and boot targets</span></button><button role="tab" aria-selected={tab === "autoexec"} className={tab === "autoexec" ? "active" : ""} onClick={() => setTab("autoexec")}>autoexec.ipxe<span>First-stage chainloader</span></button></div>
      <div className="code-window"><div className="code-titlebar"><span /><span /><span /><strong>{tab}.ipxe</strong></div><pre>{loading ? "Loading script…" : content}</pre></div>
    </section>
  );
}

function ActivityView({ events }: { events: PxeEvent[] }) {
  const [query, setQuery] = useState("");
  const [protocol, setProtocol] = useState<"All" | Protocol>("All");
  const [paused, setPaused] = useState(false);
  const [snapshot, setSnapshot] = useState(events);
  useEffect(() => { if (!paused) setSnapshot(events); }, [events, paused]);
  const visible = snapshot.filter((event) => (protocol === "All" || event.protocol === protocol) && `${event.protocol} ${event.level} ${event.message}`.toLowerCase().includes(query.toLowerCase())).slice().reverse();
  return (
    <section className="panel activity-panel">
      <div className="toolbar"><div><p className="eyebrow">Live protocol feed</p><h2>Server activity</h2><p>Up to 2,000 DHCP, TFTP, HTTP, and system events.</p></div><Button onClick={() => setPaused((value) => !value)}>{paused ? <Play size={16} /> : <Pause size={16} />}{paused ? "Resume" : "Pause"}</Button></div>
      <div className="filterbar"><label className="search-field"><Search size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search event messages" /></label><div className="protocol-filters">{(["All", "Dhcp", "Tftp", "Http", "System"] as const).map((item) => <button className={item === protocol ? "active" : ""} onClick={() => setProtocol(item)} key={item}>{item}</button>)}</div></div>
      <div className="event-stream">{visible.map((event, index) => <EventRow event={event} key={`${event.timestamp}-${index}`} />)}{visible.length === 0 && <InlineEmpty text="No events match the current filters." />}</div>
    </section>
  );
}

function AddImageDialog({ open, onOpenChange, disabled, presets, loadPresets, pending, onPreset, onUrl, onUploaded, notify }: { open: boolean; onOpenChange: (open: boolean) => void; disabled: boolean; presets: ImagePreset[]; loadPresets: () => Promise<void>; pending: string | null; onPreset: (preset: ImagePreset) => void; onUrl: (input: DownloadImageInput) => void; onUploaded: () => void; notify: (message: string, tone?: ToastTone) => void }) {
  const [tab, setTab] = useState<"preset" | "url" | "upload">("preset");
  const [category, setCategory] = useState("custom");
  const [filename, setFilename] = useState("");
  const [url, setUrl] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [progress, setProgress] = useState(0);
  useEffect(() => { if (open && presets.length === 0) void loadPresets().catch((error) => notify(error instanceof Error ? error.message : "Presets could not be loaded.", "error")); }, [open, presets.length, loadPresets, notify]);
  const submitUrl = () => { if (!filename || !url) { notify("Enter both a filename and source URL.", "error"); return; } onUrl({ category: category || "custom", filename, url }); };
  const upload = async () => { if (!file) return; setProgress(1); try { await dashboardTransport.uploadImage(file, category || "custom", setProgress); onUploaded(); setFile(null); setProgress(0); } catch (error) { notify(error instanceof Error ? error.message : "Upload failed.", "error"); setProgress(0); } };
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal><Dialog.Overlay className="dialog-overlay" /><Dialog.Content className="dialog-content" aria-describedby="add-image-description"><div className="dialog-header"><div><Dialog.Title>Add boot image</Dialog.Title><Dialog.Description id="add-image-description">Install a curated preset, download from a URL, or upload a local file.</Dialog.Description></div><Dialog.Close asChild><Button size="icon" variant="ghost"><X size={18} /></Button></Dialog.Close></div>
        <div className="dialog-tabs">{(["preset", "url", "upload"] as const).map((item) => <button className={tab === item ? "active" : ""} onClick={() => setTab(item)} key={item}>{item === "preset" ? "Official presets" : item === "url" ? "Download URL" : "Upload file"}</button>)}</div>
        <div className="dialog-body">
          {tab === "preset" && <div className="preset-grid">{presets.map((preset) => <article className="preset-card" key={preset.id}><div className="preset-icon"><PackagePlus size={19} /></div><div><strong>{preset.name}</strong><p>{preset.description}</p><span>{preset.size_estimate} · {preset.category}</span></div><Button size="sm" variant="primary" disabled={disabled || pending === `preset-${preset.id}`} onClick={() => onPreset(preset)}>{pending === `preset-${preset.id}` ? "Scheduling…" : "Install"}</Button></article>)}</div>}
          {tab === "url" && <div className="form-stack"><label>Category<input value={category} onChange={(event) => setCategory(event.target.value)} placeholder="custom" /></label><label>Filename<input value={filename} onChange={(event) => setFilename(event.target.value)} placeholder="rescue.iso" /></label><label>Source URL<input type="url" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://example.com/rescue.iso" /></label><div className="dialog-actions"><Button variant="primary" disabled={disabled || pending === "url-download"} onClick={submitUrl}><Download size={16} /> Schedule download</Button></div></div>}
          {tab === "upload" && <div className="form-stack"><label>Destination category<input value={category} onChange={(event) => setCategory(event.target.value)} placeholder="custom" /></label><label className="dropzone"><Upload size={25} /><strong>{file ? file.name : "Choose a boot image"}</strong><span>{file ? formatBytes(file.size) : "ISO, IMG, kernel, initrd, or EFI binary"}</span><input type="file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label>{progress > 0 && <div className="progress"><span style={{ width: `${progress}%` }} /></div>}<div className="dialog-actions"><Button variant="primary" disabled={disabled || !file || progress > 0} onClick={() => void upload()}><FileUp size={16} /> Upload image</Button></div></div>}
        </div>
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
  );
}

function ClientCompact({ client }: { client: ClientSnapshot }) {
  return <div className="compact-item"><div className="client-icon"><Server size={16} /></div><div className="compact-main"><strong className="mono">{client.mac}</strong><span>{client.ip ?? "Acquiring address"} · {client.arch}</span></div><span className={cn("stage-badge", `stage-${stageTone(client.stage_text)}`)}>{client.stage_text}</span><small>{client.last_seen_secs_ago}s</small></div>;
}

function EventRow({ event }: { event: PxeEvent }) {
  return <div className={cn("event-row", `event-${event.level.toLowerCase()}`)}><span className="event-time mono">{event.timestamp}</span><span className={protocolClass(event.protocol)}>{event.protocol.toUpperCase()}</span><span className="event-message">{event.message}</span></div>;
}

function PanelHeader({ title, eyebrow, action }: { title: string; eyebrow?: string; action?: React.ReactNode }) {
  return <div className="panel-header"><div><h3>{title}</h3>{eyebrow && <span>{eyebrow}</span>}</div>{action}</div>;
}

function EmptyState({ icon: Icon, title, body, action }: { icon: typeof Server; title: string; body: string; action?: React.ReactNode }) {
  return <div className="empty-state"><div><Icon size={25} /></div><h3>{title}</h3><p>{body}</p>{action}</div>;
}

function InlineEmpty({ text }: { text: string }) { return <div className="inline-empty">{text}</div>; }

function LoadingState() {
  return <div className="loading-grid"><div className="skeleton hero-skeleton" />{Array.from({ length: 5 }).map((_, index) => <div className="skeleton metric-skeleton" key={index} />)}<div className="skeleton panel-skeleton" /><div className="skeleton panel-skeleton" /></div>;
}
