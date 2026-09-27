import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface EndpointItem {
  id: string;
  title: string;
  url: string;
  zoom_factor?: number;
  muted: boolean;
  reload_interval_minutes?: number;
}

interface KioskConfig {
  version: number;
  window: {
    fullscreen: boolean;
    decorations: boolean;
    background_color: string;
    reserved_header_height_px: number;
  };
  layout: {
    strategy: string;
    target_tile_aspect_ratio: number;
    rows?: number[];
    padding_px: number;
    gap_px: number;
  };
  timing: {
    grid_view_duration_ms: number;
    maximized_hold_duration_ms: number;
    transition_duration_ms: number;
    preparing_timeout_ms: number;
    user_idle_resume_ms: number;
  };
  tour: {
    auto_start: boolean;
    pause_on_interaction: boolean;
    refresh_before_maximize: boolean;
    loop_tour: boolean;
  };
  network_dns: {
    adblock_dns_enabled: boolean;
    dns_provider: string;
    doh_url: string;
    dot_url: string;
    plain_dns_ip: string;
  };
  limits: {
    max_resident_webviews: number;
  };
  endpoints: EndpointItem[];
}

interface ConfigMetaResponse {
  config: KioskConfig;
  source: "Cli" | "Portable" | "AppData" | "Defaults";
  is_readonly: boolean;
  resolved_path: string;
}

interface TourStatusPayload {
  state: "Stopped" | "GridView" | "Maximizing" | "MaximizedSingleSite" | "Minimizing" | "PreparingNext" | "Paused";
  active_index: number | null;
  active_title: string | null;
  progress_percent: number;
  is_paused: boolean;
}

// Elements
const hudOverlay = document.getElementById("hud-overlay")!;
const hudStatus = document.getElementById("hud-status")!;
const hudTitle = document.getElementById("hud-title")!;
const hudProgressBar = document.getElementById("hud-progress-bar")!;
const btnPrev = document.getElementById("btn-prev")!;
const btnPause = document.getElementById("btn-pause")!;
const btnNext = document.getElementById("btn-next")!;
const btnFullscreen = document.getElementById("btn-fullscreen")!;
const btnSettings = document.getElementById("btn-settings")!;
const settingsDrawer = document.getElementById("settings-drawer")!;
const btnCloseSettings = document.getElementById("btn-close-settings")!;
const sourceBadge = document.getElementById("source-badge")!;
const btnSaveSettings = document.getElementById("btn-save-settings") as HTMLButtonElement;
const btnExportSettings = document.getElementById("btn-export-settings") as HTMLButtonElement;
const endpointsList = document.getElementById("endpoints-list")!;

// State
let currentConfigMeta: ConfigMetaResponse | null = null;
let isPaused = false;
let hudTimeout: number | null = null;

// Show HUD on mouse move near top
window.addEventListener("mousemove", (e) => {
  if (e.clientY <= 50 || !settingsDrawer.classList.contains("hidden")) {
    showHud();
  } else {
    scheduleHideHud();
  }
});

function showHud() {
  if (hudTimeout) {
    clearTimeout(hudTimeout);
    hudTimeout = null;
  }
  hudOverlay.classList.remove("hidden");
}

function scheduleHideHud() {
  if (hudTimeout) clearTimeout(hudTimeout);
  hudTimeout = window.setTimeout(() => {
    if (settingsDrawer.classList.contains("hidden")) {
      hudOverlay.classList.add("hidden");
    }
  }, 2500);
}

// Controls
btnPause.addEventListener("click", () => togglePause());
btnNext.addEventListener("click", () => invoke("next_tile"));
btnPrev.addEventListener("click", () => invoke("prev_tile"));
btnFullscreen.addEventListener("click", () => invoke("toggle_fullscreen"));
btnSettings.addEventListener("click", () => openSettings());
btnCloseSettings.addEventListener("click", () => closeSettings());

async function togglePause() {
  isPaused = !isPaused;
  btnPause.innerHTML = isPaused ? "&#9654;" : "&#10074;&#10074;";
  await invoke(isPaused ? "pause_tour" : "resume_tour");
}

function openSettings() {
  settingsDrawer.classList.remove("hidden");
  showHud();
  loadConfigIntoDrawer();
}

function closeSettings() {
  settingsDrawer.classList.add("hidden");
  scheduleHideHud();
}

async function loadConfigIntoDrawer() {
  try {
    const meta: ConfigMetaResponse = await invoke("get_config");
    currentConfigMeta = meta;

    sourceBadge.textContent = `Source: ${meta.source} (${meta.resolved_path})`;
    if (meta.is_readonly) {
      sourceBadge.classList.add("readonly");
      sourceBadge.textContent += " - [READ ONLY]";
      btnSaveSettings.disabled = true;
      btnSaveSettings.title = "Active config is locked by CLI or Portable override. Use Export instead.";
    } else {
      sourceBadge.classList.remove("readonly");
      btnSaveSettings.disabled = false;
      btnSaveSettings.title = "";
    }

    const cfg = meta.config;
    (document.getElementById("cfg-target-aspect") as HTMLInputElement).value = cfg.layout.target_tile_aspect_ratio.toString();
    (document.getElementById("cfg-grid-duration") as HTMLInputElement).value = cfg.timing.grid_view_duration_ms.toString();
    (document.getElementById("cfg-max-duration") as HTMLInputElement).value = cfg.timing.maximized_hold_duration_ms.toString();
    (document.getElementById("cfg-transition-duration") as HTMLInputElement).value = cfg.timing.transition_duration_ms.toString();
    (document.getElementById("cfg-adblock-dns") as HTMLInputElement).checked = cfg.network_dns.adblock_dns_enabled;

    endpointsList.innerHTML = "";
    cfg.endpoints.forEach((ep) => {
      const row = document.createElement("div");
      row.className = "endpoint-row";
      row.innerHTML = `
        <div class="endpoint-info">
          <span class="endpoint-title">${ep.title}</span>
          <span class="endpoint-url">${ep.url}</span>
        </div>
      `;
      endpointsList.appendChild(row);
    });
  } catch (err) {
    console.error("Failed to load config:", err);
  }
}

btnSaveSettings.addEventListener("click", async () => {
  if (!currentConfigMeta || currentConfigMeta.is_readonly) return;
  const cfg = currentConfigMeta.config;
  cfg.layout.target_tile_aspect_ratio = parseFloat((document.getElementById("cfg-target-aspect") as HTMLInputElement).value);
  cfg.timing.grid_view_duration_ms = parseInt((document.getElementById("cfg-grid-duration") as HTMLInputElement).value, 10);
  cfg.timing.maximized_hold_duration_ms = parseInt((document.getElementById("cfg-max-duration") as HTMLInputElement).value, 10);
  cfg.timing.transition_duration_ms = parseInt((document.getElementById("cfg-transition-duration") as HTMLInputElement).value, 10);
  cfg.network_dns.adblock_dns_enabled = (document.getElementById("cfg-adblock-dns") as HTMLInputElement).checked;

  try {
    await invoke("save_config", { newConfig: cfg });
    alert("Configuration saved successfully.");
    closeSettings();
  } catch (err) {
    alert("Error saving config: " + err);
  }
});

btnExportSettings.addEventListener("click", async () => {
  if (!currentConfigMeta) return;
  const path = prompt("Enter path to export configuration:", "kiosk-config.json");
  if (path) {
    try {
      await invoke("export_config", { config: currentConfigMeta.config, destinationPath: path });
      alert("Configuration exported to " + path);
    } catch (err) {
      alert("Error exporting config: " + err);
    }
  }
});

// Keyboard shortcuts
window.addEventListener("keydown", (e) => {
  if (e.target instanceof HTMLInputElement) return;
  if (e.code === "Space") {
    e.preventDefault();
    togglePause();
  } else if (e.code === "ArrowRight") {
    invoke("next_tile");
  } else if (e.code === "ArrowLeft") {
    invoke("prev_tile");
  } else if (e.code === "F11") {
    e.preventDefault();
    invoke("toggle_fullscreen");
  } else if (e.code === "Escape") {
    if (!settingsDrawer.classList.contains("hidden")) {
      closeSettings();
    } else {
      invoke("minimize_current");
    }
  }
});

// Listen to backend tour updates
listen<TourStatusPayload>("tour-status-update", (event) => {
  const p = event.payload;
  isPaused = p.is_paused;
  btnPause.innerHTML = isPaused ? "&#9654;" : "&#10074;&#10074;";

  if (p.state === "GridView") {
    hudStatus.textContent = "GRID VIEW";
    hudStatus.classList.remove("maximized");
    hudTitle.textContent = "Ambient Overview";
  } else if (p.state === "MaximizedSingleSite" || p.state === "Maximizing") {
    hudStatus.textContent = "MAXIMIZED";
    hudStatus.classList.add("maximized");
    hudTitle.textContent = p.active_title || `Site #${(p.active_index ?? 0) + 1}`;
  } else if (p.state === "PreparingNext") {
    hudStatus.textContent = "PREPARING";
    hudStatus.classList.remove("maximized");
    hudTitle.textContent = `Pre-refreshing ${p.active_title || "next site"}...`;
  }

  hudProgressBar.style.width = `${Math.min(100, Math.max(0, p.progress_percent))}%`;
});

// Initial boot
showHud();
scheduleHideHud();
