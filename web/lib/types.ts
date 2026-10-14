export type EventLevel = "Info" | "Success" | "Warn" | "Error";
export type Protocol = "Dhcp" | "Tftp" | "Http" | "System";

export type BootStage =
  | "Discovering"
  | "Booted"
  | { Offered: string }
  | { Acked: string }
  | { TftpTransfer: { file: string; percent: number } }
  | { HttpBoot: string };

export interface PxeEvent {
  timestamp: string;
  level: EventLevel;
  protocol: Protocol;
  message: string;
}

export interface ClientSnapshot {
  mac: string;
  ip: string | null;
  arch: string;
  stage: BootStage;
  stage_text: string;
  last_seen_secs_ago: number;
}

export interface MetricsSnapshot {
  dhcp_in: number;
  dhcp_out: number;
  tftp_bytes: number;
  tftp_files: number;
  http_requests: number;
  errors: number;
}

export interface StatusSnapshot {
  server_ip: string;
  http_port: number;
  uptime_secs: number;
  metrics: MetricsSnapshot;
  clients: ClientSnapshot[];
  events: PxeEvent[];
}

export interface BootImage {
  name: string;
  rel_path: string;
  category: string;
  size_bytes: number;
  size_formatted: string;
  is_iso: boolean;
  is_kernel: boolean;
  is_initrd: boolean;
}

export interface PresetFile {
  url: string;
  filename: string;
}

export interface ImagePreset {
  id: string;
  name: string;
  category: string;
  description: string;
  size_estimate: string;
  files: PresetFile[];
}

export interface DownloadImageInput {
  category: string;
  filename: string;
  url: string;
}

export interface DashboardTransport {
  readonly mode: "demo" | "live";
  getStatus(signal?: AbortSignal): Promise<StatusSnapshot>;
  getImages(signal?: AbortSignal): Promise<BootImage[]>;
  getPresets(signal?: AbortSignal): Promise<ImagePreset[]>;
  getScript(kind: "boot" | "autoexec", signal?: AbortSignal): Promise<string>;
  disconnectClient(id: string): Promise<void>;
  clearClients(): Promise<void>;
  downloadPreset(id: string): Promise<void>;
  downloadImage(input: DownloadImageInput): Promise<void>;
  uploadImage(file: File, category: string, onProgress: (percent: number) => void): Promise<void>;
}
