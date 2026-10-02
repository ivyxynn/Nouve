import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { Bridge, onEvent } from "../core/bridge";
import { h, svg } from "./dom";
import { ICONS } from "./icons";

// One live terminal, kept at module level so it survives re-renders. Minimize
// leaves the terminal view for the overview (the pills), and the shell keeps
// running: coming back re-attaches this same instance with its history intact.
// Only Close (or `exit` in the shell) actually ends the process.
const TERM_ID = "omp-session";

let term: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let cachedWrapper: HTMLElement | null = null;
let body: HTMLElement | null = null;
let unlistenOutput: (() => void) | null = null;
let unlistenExit: (() => void) | null = null;
let exited = false;
/** How the terminal view was left: Minimize keeps the shell, Close kills it. */
export type LeaveReason = "minimize" | "close";
let onLeave: ((reason: LeaveReason) => void) | null = null;
let resizeObserver: ResizeObserver | null = null;
/** Last cols/rows sent to Rust, so a burst of ResizeObserver ticks is one call. */
let lastSize = { cols: 0, rows: 0 };

const BTN_STYLE =
  "display:flex;align-items:center;justify-content:center;width:20px;height:20px;" +
  "border-radius:5px;border:1px solid #1f293d;background:#0f1729;color:#8ba2c4;" +
  "cursor:pointer;padding:0;transition:background .12s,color .12s;";

function iconButton(icon: string, title: string, onClick: () => void): HTMLButtonElement {
  const b = h("button", { type: "button", title, style: BTN_STYLE }, svg(icon, 11)) as HTMLButtonElement;
  b.addEventListener("mouseenter", () => {
    b.style.background = "#16233d";
    b.style.color = "#e2ecfc";
  });
  b.addEventListener("mouseleave", () => {
    b.style.background = "#0f1729";
    b.style.color = "#8ba2c4";
  });
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  return b;
}

/** Refit the PTY to the container and tell Rust the new size. */
function fit() {
  if (!term || !fitAddon || !cachedWrapper?.isConnected) return;
  try {
    fitAddon.fit();
  } catch {
    // The host can be 0×0 while the island animates; fit throws and we retry on
    // the next resize tick.
    return;
  }
  // A resize animation fires this many times a frame; only tell Rust when the
  // cell grid actually changed.
  if (term.cols === lastSize.cols && term.rows === lastSize.rows) return;
  lastSize = { cols: term.cols, rows: term.rows };
  void Bridge.terminalResize(TERM_ID, term.cols, term.rows);
}
/** Drops the xterm instance and its listeners. Does not touch the Rust session. */
function teardown() {
  unlistenOutput?.();
  unlistenExit?.();
  unlistenOutput = null;
  unlistenExit = null;
  resizeObserver?.disconnect();
  resizeObserver = null;
  term?.dispose();
  term = null;
  fitAddon = null;
  cachedWrapper = null;
  body = null;
  exited = false;
  lastSize = { cols: 0, rows: 0 };
}

/**
 * @param onLeaveCb Called whenever the terminal view should close and the island
 *   go back to the overview — both Minimize and Close end here, and the reason
 *   says whether the shell is still alive. Minimize leaves the shell running;
 *   Close kills it first.
 */
export function terminalCard(onLeaveCb: (reason: LeaveReason) => void): HTMLElement {
  onLeave = onLeaveCb;
  // Re-attach the live terminal rather than starting a new one. It was detached
  // from the DOM when we left, so refit + repaint once it is back in.
  if (cachedWrapper) {
    requestAnimationFrame(() => {
      fit();
      term?.refresh(0, term.rows - 1);
    });
    return cachedWrapper;
  }

  const dotEl = h("span", {
    style: "display:inline-block;width:8px;height:8px;border-radius:50%;background:#4caf50;",
  });
  const minimizeBtn = iconButton(ICONS.minus, "Minimize", minimize);
  const closeBtn = iconButton(ICONS.xmark, "Close terminal", closeTerminal);

  const header = h(
    "div",
    {
      style:
        "display:flex;align-items:center;justify-content:space-between;padding:6px 12px;" +
        "background:#0d1527;border-bottom:1px solid #1f293d;font-size:11px;color:#8ba2c4;font-family:monospace;",
    },
    h(
      "div",
      { style: "display:flex;align-items:center;gap:6px;" },
      dotEl,
      h("b", { text: "Terminal (OMP)", style: "color:#e2ecfc;" }),
    ),
    h(
      "div",
      { style: "display:flex;align-items:center;gap:8px;" },
      h("span", { text: "PowerShell / CMD", style: "opacity:0.7;" }),
      minimizeBtn,
      closeBtn,
    ),
  );

  body = h("div", {
    style: "flex:1 1 auto;width:100%;min-height:160px;padding:6px;overflow:hidden;",
  });

  const wrapper = h(
    "div",
    {
      class: "term-card",
      style:
        "display:flex;flex-direction:column;width:100%;height:100%;background:#090d16;" +
        "border-radius:12px;overflow:hidden;border:1px solid #1f293d;" +
        "box-shadow:0 4px 20px rgba(0,0,0,0.5);margin:0;padding:0;",
    },
    header,
    body,
  );
  cachedWrapper = wrapper;

  const host = body;
  requestAnimationFrame(() => {
    void setup(host, dotEl);
  });

  return wrapper;
}

async function setup(host: HTMLElement, dotEl: HTMLElement) {
  const t = new Terminal({
    theme: {
      background: "#090d16",
      foreground: "#e2ecfc",
      cursor: "#4caf50",
      selectionBackground: "rgba(76, 175, 80, 0.3)",
    },
    fontFamily: "Consolas, 'Cascadia Code', monospace",
    fontSize: 12,
    cursorBlink: true,
    convertEol: true,
    rows: 14,
    cols: 60,
  });
  const fa = new FitAddon();
  t.loadAddon(fa);
  t.open(host);
  term = t;
  fitAddon = fa;
  try {
    fa.fit();
  } catch {}

  await Bridge.terminalOpen(TERM_ID, undefined, t.cols, t.rows);

  t.onData((data) => {
    if (exited) return;
    void Bridge.terminalWrite(TERM_ID, data);
  });

  unlistenOutput = await onEvent<{ id: string; data: string }>("terminal://output", (ev) => {
    if (ev.id === TERM_ID) t.write(ev.data);
  });

  // The shell ends on its own — the user typed exit, or it crashed. Say so, free
  // the Rust session, then hand the view back so we're not staring at a corpse.
  unlistenExit = await onEvent<{ id: string; code: number }>("terminal://exit", (ev) => {
    if (ev.id !== TERM_ID || exited) return;
    exited = true;
    dotEl.style.background = "#8ba2c4";
    t.write(`\r\n\x1b[90m[process exited with code ${ev.code}]\x1b[0m\r\n`);
    void Bridge.terminalClose(TERM_ID);
    window.setTimeout(() => {
      if (exited) {
        teardown();
        onLeave?.("close");
      }
    }, 1500);
  });

  // Keep the PTY in step with the panel. Re-attaching after a re-render also
  // lands here, which is what re-fits the terminal at its new size.
  const ro = new ResizeObserver(() => fit());
  ro.observe(host);
  resizeObserver = ro;

  t.focus();
}

/**
 * Minimize: leave the terminal view and go back to the overview (the pills),
 * exactly like clicking another pill. The shell keeps running — this only
 * detaches the view, so coming back to the Terminal pill re-attaches the same
 * session with its scrollback intact.
 */
function minimize() {
  onLeave?.("minimize");
}

/** Close: kill the shell for real, then hand the view back to the overview. */
function closeTerminal() {
  void Bridge.terminalClose(TERM_ID);
  teardown();
  onLeave?.("close");
}
