import { mockImages, mockPresets, mockScripts, mockStatus } from "./mock-data";
import type { DashboardTransport, DownloadImageInput } from "./types";

async function responseOrThrow(response: Response) {
  if (!response.ok) {
    const message = await response.text().catch(() => "");
    throw new Error(message || `Request failed with HTTP ${response.status}`);
  }
  return response;
}

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

class MockDashboardTransport implements DashboardTransport {
  readonly mode = "demo" as const;

  async getStatus() { await wait(180); return structuredClone(mockStatus); }
  async getImages() { await wait(160); return structuredClone(mockImages); }
  async getPresets() { await wait(140); return structuredClone(mockPresets); }
  async getScript(kind: "boot" | "autoexec") { await wait(120); return mockScripts[kind]; }

  private readonly() { throw new Error("Actions are disabled in the Vercel demo."); }
  async disconnectClient() { this.readonly(); }
  async clearClients() { this.readonly(); }
  async downloadPreset() { this.readonly(); }
  async downloadImage() { this.readonly(); }
  async uploadImage() { this.readonly(); }
}

class SameOriginHttpTransport implements DashboardTransport {
  readonly mode = "live" as const;

  async getStatus(signal?: AbortSignal) {
    return responseOrThrow(await fetch("/api/status", { cache: "no-store", signal })).then((r) => r.json());
  }
  async getImages(signal?: AbortSignal) {
    return responseOrThrow(await fetch("/api/images", { cache: "no-store", signal })).then((r) => r.json());
  }
  async getPresets(signal?: AbortSignal) {
    return responseOrThrow(await fetch("/api/images/presets", { cache: "no-store", signal })).then((r) => r.json());
  }
  async getScript(kind: "boot" | "autoexec", signal?: AbortSignal) {
    const path = kind === "boot" ? "/boot.ipxe" : "/autoexec.ipxe";
    return responseOrThrow(await fetch(path, { cache: "no-store", signal })).then((r) => r.text());
  }
  async disconnectClient(id: string) {
    await responseOrThrow(await fetch("/api/clients/disconnect", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id }),
    }));
  }
  async clearClients() {
    await responseOrThrow(await fetch("/api/clients/clear", { method: "POST" }));
  }
  async downloadPreset(id: string) {
    await responseOrThrow(await fetch("/api/images/download-preset", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id }),
    }));
  }
  async downloadImage(input: DownloadImageInput) {
    await responseOrThrow(await fetch("/api/images/download", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(input),
    }));
  }
  uploadImage(file: File, category: string, onProgress: (percent: number) => void) {
    return new Promise<void>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      const query = new URLSearchParams({ category, filename: file.name });
      xhr.open("POST", `/api/images/upload?${query}`);
      xhr.upload.onprogress = (event) => event.lengthComputable && onProgress(Math.round((event.loaded / event.total) * 100));
      xhr.onload = () => xhr.status >= 200 && xhr.status < 300 ? resolve() : reject(new Error(xhr.responseText || `Upload failed with HTTP ${xhr.status}`));
      xhr.onerror = () => reject(new Error("Upload failed because the server connection was lost."));
      xhr.send(file);
    });
  }
}

export const dashboardTransport: DashboardTransport =
  process.env.NEXT_PUBLIC_DASHBOARD_MODE === "live"
    ? new SameOriginHttpTransport()
    : new MockDashboardTransport();
