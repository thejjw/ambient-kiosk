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

import { createEndpointRowElement, renderEndpointsList, renderDiagnostics, EndpointItem, DiagnosticsSnapshot } from "./main";

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
