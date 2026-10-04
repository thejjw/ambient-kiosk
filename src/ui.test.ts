import { expect, test, describe, beforeEach } from "bun:test";
import { Window } from "happy-dom";

const happyWindow = new Window();
const happyDoc = happyWindow.document;

// Assign DOM globals for test execution without bare any
const globalScope = globalThis as unknown as {
  window: Window;
  document: typeof happyDoc;
};
globalScope.window = happyWindow;
globalScope.document = happyDoc;

import { createEndpointRowElement, renderEndpointsList, renderDiagnostics, initWindowControls, renderTourStatus, EndpointItem, DiagnosticsSnapshot, TourStatusPayload } from "./main";

test("paused badge replaces maximized status and resume restores view and action labels", () => {
  happyDoc.body.innerHTML = '<span id="hud-status"></span><span id="hud-title"></span><button id="btn-pause"></button><div id="hud-progress-bar"></div>';
  const payload: TourStatusPayload = { state: "MaximizedSingleSite", active_index: 0, active_title: "Test feed", progress_percent: 40, is_paused: false };
  renderTourStatus(payload);
  renderTourStatus({ ...payload, state: "Paused", is_paused: true });
  expect(happyDoc.getElementById("hud-status")!.textContent).toBe("PAUSED");
  expect(happyDoc.getElementById("hud-title")!.textContent).toBe("Test feed");
  expect(happyDoc.getElementById("btn-pause")!.getAttribute("aria-label")).toBe("Resume tour");
  renderTourStatus(payload);
  expect(happyDoc.getElementById("hud-status")!.textContent).toBe("MAXIMIZED");
  expect(happyDoc.getElementById("btn-pause")!.getAttribute("aria-label")).toBe("Pause tour");
  renderTourStatus({ ...payload, state: "GridView" });
  renderTourStatus({ ...payload, state: "Paused", is_paused: true, active_title: null });
  expect(happyDoc.getElementById("hud-status")!.textContent).toBe("PAUSED");
  expect(happyDoc.getElementById("hud-title")!.textContent).toBe("Ambient Overview");
  renderTourStatus({ ...payload, state: "GridView" });
  expect(happyDoc.getElementById("hud-status")!.textContent).toBe("GRID VIEW");
});

describe("Endpoint UI Security & Validation", () => {
  beforeEach(() => {
    happyDoc.body.innerHTML = `
      <div id="endpoints-list"></div>
      <button id="btn-add-endpoint"></button>
    `;
  });

  test("hostile-string injection is rendered safely as property without DOM element creation", () => {
    const hostileTitle = "<script>alert('xss')</script>";
    const hostileUrl = "https://example.com/?q=<img src=x onerror=alert(1)>";

    const ep: EndpointItem = {
      id: "test-hostile",
      title: hostileTitle,
      url: hostileUrl,
      muted: true,
    };

    const row = createEndpointRowElement(
      ep,
      0,
      1,
      false,
      () => {},
      () => {},
      () => {}
    );

    // Assert literal value in input element
    const inputs = row.querySelectorAll("input");
    expect(inputs.length).toBe(2);
    expect(inputs[0].value).toBe(hostileTitle);
    expect(inputs[1].value).toBe(hostileUrl);

    // CRITICAL SECURITY ASSERTION: No <script> or <img> tags were injected into the DOM!
    expect(row.querySelectorAll("script").length).toBe(0);
    expect(row.querySelectorAll("img").length).toBe(0);
    expect(row.innerHTML).not.toContain("<script>");
  });

  test("minimum endpoint limit: delete button disabled when endpoints.length <= 1", () => {
    const endpoints: EndpointItem[] = [
      { id: "1", title: "Single Feed", url: "https://single.com", muted: true },
    ];

    renderEndpointsList(endpoints, false, 12);
    const deleteBtn = happyDoc.querySelector(".endpoint-btn.delete") as HTMLButtonElement;
    expect(deleteBtn).not.toBeNull();
    expect(deleteBtn.disabled).toBe(true);
    expect(deleteBtn.title).toContain("At least 1 endpoint required");
  });

  test("maximum resident limit: add button disabled when endpoints.length >= maxResidents", () => {
    const endpoints: EndpointItem[] = [
      { id: "1", title: "Site 1", url: "https://1.com", muted: true },
      { id: "2", title: "Site 2", url: "https://2.com", muted: true },
    ];

    const maxLimit = 2;
    renderEndpointsList(endpoints, false, maxLimit);
    const addBtn = happyDoc.getElementById("btn-add-endpoint") as HTMLButtonElement;
    expect(addBtn.disabled).toBe(true);
    expect(addBtn.title).toContain("Maximum limit of 2 endpoints reached");
  });

  test("read-only locked mode disables all input fields and modification buttons", () => {
    const endpoints: EndpointItem[] = [
      { id: "1", title: "Site 1", url: "https://1.com", muted: true },
      { id: "2", title: "Site 2", url: "https://2.com", muted: true },
    ];

    renderEndpointsList(endpoints, true, 12); // isReadonly = true

    const inputs = happyDoc.querySelectorAll<HTMLInputElement>(".endpoint-inputs input");
    inputs.forEach((input) => {
      expect(input.disabled).toBe(true);
    });

    const buttons = happyDoc.querySelectorAll<HTMLButtonElement>(".endpoint-btn");
    buttons.forEach((btn) => {
      expect(btn.disabled).toBe(true);
    });

    const addBtn = happyDoc.getElementById("btn-add-endpoint") as HTMLButtonElement;
    expect(addBtn.disabled).toBe(true);
    expect(addBtn.title).toContain("Configuration is read-only");
  });

  test("reordering updates endpoints array deterministically", () => {
    const endpoints: EndpointItem[] = [
      { id: "1", title: "Site A", url: "https://a.com", muted: true },
      { id: "2", title: "Site B", url: "https://b.com", muted: true },
    ];

    renderEndpointsList(endpoints, false, 12);

    // Row 0 has move-down button enabled, move-up disabled
    const rows = happyDoc.querySelectorAll(".endpoint-row");
    const row0DownBtn = rows[0].querySelectorAll("button")[1];
    expect(row0DownBtn.disabled).toBe(false);

    // Click move down on Site A
    row0DownBtn.click();

    // Endpoints array swapped
    expect(endpoints[0].title).toBe("Site B");
    expect(endpoints[1].title).toBe("Site A");
  });
});

describe("Operational diagnostics UI", () => {
  beforeEach(() => {
    happyDoc.body.innerHTML = '<div id="diagnostics-health"></div><div id="diagnostics-feeds"></div>';
  });

  test("shows refresh status and logging warnings without interpreting endpoint IDs as markup", () => {
    const snapshot: DiagnosticsSnapshot = {
      session_id: "session-test",
      uptime_seconds: 65,
      log_path: "C:\\logs\\ambient-kiosk.jsonl",
      storage_available: false,
      write_errors: 1,
      dropped_records: 2,
      tour_state: "GridView",
      active_endpoint_id: null,
      proxy_enabled: true,
      proxy_connections: 3,
      dns_resolved: 4,
      dns_blocked: 1,
      dns_failed: 0,
      feeds: [{
        endpoint_id: '<img src=x onerror="alert(1)">',
        tile_index: 0,
        last_attempt_at_ms: null,
        last_completed_at_ms: null,
        last_outcome: null,
        elapsed_ms: null,
        pending: false,
      }],
    };

    renderDiagnostics(snapshot);

    expect(happyDoc.getElementById("diagnostics-health")?.textContent).toContain("Logging warning");
    expect(happyDoc.getElementById("diagnostics-feeds")?.textContent).toContain("Not attempted this session");
    expect(happyDoc.getElementById("diagnostics-feeds")?.textContent).toContain(snapshot.feeds[0].endpoint_id);
    expect(happyDoc.querySelector("#diagnostics-feeds img")).toBeNull();
  });
});

describe("HUD window controls", () => {
  beforeEach(() => {
    happyDoc.body.innerHTML = `
      <div id="hud-overlay"><button id="btn-minimize-app"></button><button id="btn-close-app"></button></div>
      <div id="settings-drawer"><input id="unsaved" value="keep my edit"></div>
      <div id="window-action-error" hidden></div>
      <div id="close-confirmation" hidden><button id="btn-cancel-close">Cancel</button><button id="btn-confirm-close">Close</button><div id="close-dialog-error" hidden></div></div>`;
  });

  test("dispatches window commands, coalesces requests, and permits retry after safe failure", async () => {
    const calls: string[] = [];
    const errors: boolean[] = [];
    let release: (() => void) | undefined;
    const controls = initWindowControls(async (command) => {
      calls.push(command);
      if (command === "request_close_app") await new Promise<void>((resolve) => { release = resolve; });
      else throw new Error("private host and credentials");
    }, () => {}, (shown) => { errors.push(shown); });
    happyDoc.getElementById("btn-close-app")!.click();
    happyDoc.getElementById("btn-close-app")!.click();
    expect(calls).toEqual(["request_close_app"]);
    release!();
    await Promise.resolve();
    await Promise.resolve();
    happyDoc.getElementById("btn-minimize-app")!.click();
    await Promise.resolve();
    await Promise.resolve();
    expect(happyDoc.getElementById("window-action-error")!.textContent).toBe("Could not minimize the app. Try again.");
    expect(happyDoc.body.textContent).not.toContain("credentials");
    expect(controls.isErrorVisible()).toBe(true);
    expect(errors.at(-1)).toBe(true);
    happyDoc.getElementById("btn-minimize-app")!.click();
    expect(calls.filter((command) => command === "minimize_app").length).toBe(2);
    expect(errors.at(-1)).toBe(false);
    await Promise.resolve();
    controls.dispose();
  });

  test("defaults to Cancel, traps focus, and Escape restores edited Settings and focus", async () => {
    const calls: unknown[] = [];
    const controls = initWindowControls(async (command, args) => { calls.push([command, args]); }, () => {});
    const input = happyDoc.getElementById("unsaved")!;
    input.focus();
    controls.showConfirmation();
    controls.showConfirmation();
    const cancel = happyDoc.getElementById("btn-cancel-close")!;
    const confirm = happyDoc.getElementById("btn-confirm-close")!;
    expect(happyDoc.activeElement).toBe(cancel);
    expect(happyDoc.getElementById("settings-drawer")!.inert).toBe(true);
    happyWindow.dispatchEvent(new happyWindow.KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(happyDoc.activeElement).toBe(confirm);
    happyWindow.dispatchEvent(new happyWindow.KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true }));
    expect(happyDoc.activeElement).toBe(cancel);
    happyWindow.dispatchEvent(new happyWindow.KeyboardEvent("keydown", { code: "Escape", bubbles: true }));
    await Promise.resolve();
    expect(calls).toEqual([["resolve_close_app", { confirmed: false }]]);
    expect(controls.isOpen()).toBe(false);
    expect(happyDoc.activeElement).toBe(input);
    expect((input as unknown as HTMLInputElement).value).toBe("keep my edit");
    expect(happyDoc.getElementById("settings-drawer")!.inert).toBe(false);
    controls.dispose();
  });

  test("blocks duplicate resolutions and keeps failed confirmation available for retry", async () => {
    let reject: ((reason?: unknown) => void) | undefined;
    const calls: unknown[] = [];
    const controls = initWindowControls(async (command, args) => {
      calls.push([command, args]);
      await new Promise<void>((_, fail) => { reject = fail; });
    }, () => {});
    controls.showConfirmation();
    const confirm = happyDoc.getElementById("btn-confirm-close")!;
    confirm.click();
    confirm.click();
    expect(calls).toEqual([["resolve_close_app", { confirmed: true }]]);
    reject!(new Error("sensitive error"));
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(controls.isOpen()).toBe(true);
    expect(happyDoc.getElementById("close-dialog-error")!.textContent).toBe("Could not close the app. Try again or cancel.");
    expect(happyDoc.activeElement).toBe(happyDoc.getElementById("btn-cancel-close"));
    confirm.click();
    expect(calls.length).toBe(2);
    reject!();
    await Promise.resolve();
    await Promise.resolve();
    controls.dispose();
  });

  test("successful close stays locked while native shutdown completes", async () => {
    let calls = 0;
    const controls = initWindowControls(async () => { calls++; }, () => {});
    controls.showConfirmation();
    happyDoc.getElementById("btn-confirm-close")!.click();
    await Promise.resolve();
    happyDoc.getElementById("btn-confirm-close")!.click();
    happyWindow.dispatchEvent(new happyWindow.KeyboardEvent("keydown", { code: "Escape" }));
    expect(calls).toBe(1);
    expect((happyDoc.getElementById("btn-cancel-close") as unknown as HTMLButtonElement).disabled).toBe(true);
    controls.dispose();
  });
});
