import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface EndpointItem {
  id: string;
  title: string;
  url: string;
  zoom_factor?: number;
  muted: boolean;
  reload_interval_minutes?: number;
}

export interface KioskConfig {
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

export interface ConfigMetaResponse {
  config: KioskConfig;
  source: "Cli" | "Portable" | "AppData" | "Defaults";
  is_readonly: boolean;
  resolved_path: string;
}

export interface TourStatusPayload {
  state: "Stopped" | "GridView" | "Maximizing" | "MaximizedSingleSite" | "Minimizing" | "PreparingNext" | "Paused";
  active_index: number | null;
  active_title: string | null;
  progress_percent: number;
  is_paused: boolean;
}

export function bootstrapApp() {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return;
  }
  const currentWindow = getCurrentWindow();
  const isHud = currentWindow.label === "hud-overlay";

  if (!isHud) {
    // Main coordinator window only hosts child webviews
    document.getElementById("hud-overlay")?.remove();
    document.getElementById("settings-drawer")?.remove();
  } else {
    initHudWindow();
  }
}

if (typeof window !== "undefined") {
  bootstrapApp();
}

function initHudWindow() {
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
  const btnAddEndpoint = document.getElementById("btn-add-endpoint") as HTMLButtonElement;

  let currentConfigMeta: ConfigMetaResponse | null = null;
  let isPaused = false;
  let hudTimeout: number | null = null;

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

  async function openSettings() {
    try {
      await invoke("set_settings_open", { open: true });
    } catch (err) {
      alert("Could not open settings: " + err);
      return;
    }
    settingsDrawer.classList.remove("hidden");
    showHud();
    loadConfigIntoDrawer();
  }

  async function closeSettings() {
    try {
      await invoke("set_settings_open", { open: false });
    } catch (err) {
      alert("Could not close settings: " + err);
      return;
    }
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

      if (meta.is_readonly) {
        (document.getElementById("cfg-target-aspect") as HTMLInputElement).disabled = true;
        (document.getElementById("cfg-grid-duration") as HTMLInputElement).disabled = true;
        (document.getElementById("cfg-max-duration") as HTMLInputElement).disabled = true;
        (document.getElementById("cfg-transition-duration") as HTMLInputElement).disabled = true;
        (document.getElementById("cfg-adblock-dns") as HTMLInputElement).disabled = true;
      } else {
        (document.getElementById("cfg-target-aspect") as HTMLInputElement).disabled = false;
        (document.getElementById("cfg-grid-duration") as HTMLInputElement).disabled = false;
        (document.getElementById("cfg-max-duration") as HTMLInputElement).disabled = false;
        (document.getElementById("cfg-transition-duration") as HTMLInputElement).disabled = false;
        (document.getElementById("cfg-adblock-dns") as HTMLInputElement).disabled = false;
      }

      renderEndpointsList(cfg.endpoints, meta.is_readonly, cfg.limits.max_resident_webviews);

      btnAddEndpoint.onclick = () => {
        if (meta.is_readonly || cfg.endpoints.length >= cfg.limits.max_resident_webviews) return;
        cfg.endpoints.push({
          id: "custom-" + Date.now(),
          title: "New Website",
          url: "https://",
          muted: true,
        });
        renderEndpointsList(cfg.endpoints, meta.is_readonly, cfg.limits.max_resident_webviews);
      };
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
      alert("Saved - restart to apply changes.");
      await closeSettings();
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

  // Native shortcuts are suspended while settings are open; Escape closes the drawer.
  window.addEventListener("keydown", (e) => {
    if (e.code === "Escape" && !settingsDrawer.classList.contains("hidden")) {
      e.preventDefault();
      closeSettings();
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

  // Listen to native cursor tracking visibility event from backend
  listen<boolean>("hud-visibility", (event) => {
    if (event.payload || !settingsDrawer.classList.contains("hidden")) {
      showHud();
    } else {
      scheduleHideHud();
    }
  });

  showHud();
  scheduleHideHud();
}

export function createEndpointRowElement(
  ep: EndpointItem,
  index: number,
  total: number,
  isReadonly: boolean,
  onMoveUp: () => void,
  onMoveDown: () => void,
  onDelete: () => void
): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "endpoint-row";

  const inputsDiv = document.createElement("div");
  inputsDiv.className = "endpoint-inputs";

  const titleInput = document.createElement("input");
  titleInput.type = "text";
  titleInput.placeholder = "Website Title";
  titleInput.value = ep.title; // Safe assignment: no HTML injection
  titleInput.disabled = isReadonly;
  titleInput.addEventListener("input", () => {
    ep.title = titleInput.value;
  });

  const urlInput = document.createElement("input");
  urlInput.type = "text";
  urlInput.placeholder = "https://example.com";
  urlInput.value = ep.url; // Safe assignment: no HTML injection
  urlInput.disabled = isReadonly;
  urlInput.addEventListener("input", () => {
    ep.url = urlInput.value;
  });

  inputsDiv.appendChild(titleInput);
  inputsDiv.appendChild(urlInput);

  const actionsDiv = document.createElement("div");
  actionsDiv.className = "endpoint-actions";

  const btnUp = document.createElement("button");
  btnUp.className = "endpoint-btn";
  btnUp.textContent = "↑";
  btnUp.title = "Move Up";
  btnUp.disabled = isReadonly || index === 0;
  btnUp.addEventListener("click", onMoveUp);

  const btnDown = document.createElement("button");
  btnDown.className = "endpoint-btn";
  btnDown.textContent = "↓";
  btnDown.title = "Move Down";
  btnDown.disabled = isReadonly || index === total - 1;
  btnDown.addEventListener("click", onMoveDown);

  const btnDelete = document.createElement("button");
  btnDelete.className = "endpoint-btn delete";
  btnDelete.textContent = "✕";
  btnDelete.title = total <= 1 ? "At least 1 endpoint required" : "Delete Endpoint";
  btnDelete.disabled = isReadonly || total <= 1;
  btnDelete.addEventListener("click", onDelete);

  actionsDiv.appendChild(btnUp);
  actionsDiv.appendChild(btnDown);
  actionsDiv.appendChild(btnDelete);

  row.appendChild(inputsDiv);
  row.appendChild(actionsDiv);
  return row;
}

export function renderEndpointsList(
  endpoints: EndpointItem[],
  isReadonly: boolean,
  maxResidents: number
) {
  const list = document.getElementById("endpoints-list");
  if (!list) return;
  const addBtn = document.getElementById("btn-add-endpoint") as HTMLButtonElement | null;
  list.textContent = "";

  if (addBtn) {
    addBtn.disabled = isReadonly || endpoints.length >= maxResidents;
    if (isReadonly) {
      addBtn.title = "Configuration is read-only";
    } else if (endpoints.length >= maxResidents) {
      addBtn.title = `Maximum limit of ${maxResidents} endpoints reached`;
    } else {
      addBtn.title = "Add a new endpoint";
    }
  }

  endpoints.forEach((ep, idx) => {
    const row = createEndpointRowElement(
      ep,
      idx,
      endpoints.length,
      isReadonly,
      () => {
        if (idx > 0) {
          const tmp = endpoints[idx];
          endpoints[idx] = endpoints[idx - 1];
          endpoints[idx - 1] = tmp;
          renderEndpointsList(endpoints, isReadonly, maxResidents);
        }
      },
      () => {
        if (idx < endpoints.length - 1) {
          const tmp = endpoints[idx];
          endpoints[idx] = endpoints[idx + 1];
          endpoints[idx + 1] = tmp;
          renderEndpointsList(endpoints, isReadonly, maxResidents);
        }
      },
      () => {
        if (endpoints.length > 1) {
          endpoints.splice(idx, 1);
          renderEndpointsList(endpoints, isReadonly, maxResidents);
        }
      }
    );
    list.appendChild(row);
  });
}
