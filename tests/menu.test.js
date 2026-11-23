import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { build } from "esbuild";
import { createElement, Info } from "lucide";

const bundle = await build({ entryPoints: ["src/web/menu.js"], bundle: true, write: false, format: "esm", loader: { ".css": "text" } });
const { mountMenu } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);

test("history row icons explain exact events and retain optional Identity Lens access", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const users = [
    { id: "9007199254740993", username: "morgan", display_name: "Morgan" },
    { id: "9007199254740995", username: "other", display_name: "Other" },
  ];
  const channel = { id: "100", name: "General" };
  const state = { channel, participants: {}, events: users.map((user, index) => ({
    sequence: index + 1, user, channel, timestamp: 1000 + index, action: "left",
  })) };
  const opened = [];
  const identity = { open: (...args) => opened.push(args) };
  for (const tools of [{ identity }, {}]) {
    const menu = mountMenu(() => {}, null, null, tools);
    try {
      menu.update(state);
      menu.update(state);
      assert.equal(document.querySelectorAll(".cm-event").length, users.length);
      assert.equal(document.querySelectorAll("[data-copy-id]").length, users.length);
      for (const [index, user] of users.entries()) {
        const button = document.querySelector(`[data-copy-id="${index + 1}"]`);
        const label = `Explain event for ${user.username}`;
        assert.equal(button.getAttribute("aria-label"), label);
        assert.equal(button.title, label);
        assert.equal(button.querySelector("svg").outerHTML,
          createElement(Info, { width: 18, height: 18, "aria-hidden": "true" }).outerHTML);
        button.click();
        assert.ok(document.querySelector('[role="dialog"][aria-label="Event details"]'));
        assert.equal(document.querySelector('[data-field="user.id"] dd').textContent, user.id);
        if (tools.identity) {
          document.querySelector('[aria-label="Identity Lens"]').click();
          assert.equal(opened[index][0], user.id);
          assert.equal(opened[index][1].id, user.id);
        } else document.querySelector('[aria-label="Close Event details"]').click();
      }
    } finally { menu.dispose(); }
  }
  assert.equal(opened.length, users.length);
});

test("current-call digest reads once, reports reconnects and retention gaps, and resets on switches", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const channel = { id: "100", name: "General" };
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const base = { channel, current_call: { id: 1, channel }, participants: {}, events: [], latest_sequence: 0 };
  const menu = mountMenu(() => {});
  try {
    menu.update(base);
    const events = [
      { sequence: 1, timestamp: 1000, action: "left", user, channel },
      { sequence: 2, timestamp: 2000, action: "joined", user, channel, reconnect_of: 1, reconnect_gap_ms: 1000 },
    ];
    menu.update({ ...base, latest_sequence: 2, events });
    document.querySelector('button[aria-label="Voice history"]').click();
    assert.match(document.querySelector(".cm-digest").textContent, /0 arrivals, 0 departures, 1 probable reconnects/);
    assert.match(document.querySelector(".cm-action").textContent, /Probable reconnect/);
    document.querySelector('[aria-label="Close history"]').click();
    document.querySelector('button[aria-label="Voice history"]').click();
    assert.equal(document.querySelector(".cm-digest").hidden, true);
    document.querySelector('[aria-label="Close history"]').click();
    menu.update({ ...base, latest_sequence: 4, events: [] });
    document.querySelector('button[aria-label="Voice history"]').click();
    assert.match(document.querySelector(".cm-digest").textContent, /2 unseen events are no longer retained/);
    menu.update({ ...base, current_call: { id: 2, channel }, latest_sequence: 4 });
    assert.equal(document.querySelector(".cm-digest").hidden, true);
  } finally { menu.dispose(); }
});

test("recap control is restricted to ended private groups and includes same-group comparisons", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const channel = { id: "100", name: "Private group", kind: 3 };
  const recap = { baseline_count: 2, participant_ids: ["1", "2"], peak_participants: 2, joined_count: 0, left_count: 0, probable_reconnections: 0 };
  const call = { id: 2, channel, started_at: 3000, ended_at: 5000, recap };
  const archive = { ...call, id: 1, started_at: 1000, ended_at: 2000, events: [] };
  let watches = 0;
  const menu = mountMenu(() => {}, null, null, { watch: { open() { watches++; } } });
  try {
    const state = { channel: null, current_call: call, participants: {}, events: [], archives: [archive] };
    menu.update(state);
    assert.equal(document.querySelector('[aria-label="Call recap"]').hidden, false);
    document.querySelector('[aria-label="Call recap"]').click();
    assert.ok(document.querySelector('[aria-label="Comparison call"]'));
    document.querySelector('[aria-label="Close Call recap"]').click();
    document.querySelector('[aria-label="Voice Watch"]').click();
    assert.equal(watches, 1);
    for (const kind of [1, 2, null]) {
      menu.update({ ...state, current_call: { ...call, channel: { ...channel, kind } } });
      assert.equal(document.querySelector('[aria-label="Call recap"]').hidden, true);
    }
    menu.update({ ...state, current_call: { ...call, ended_at: null } });
    assert.equal(document.querySelector('[aria-label="Call recap"]').hidden, true);
  } finally { menu.dispose(); }
  assert.equal(document.querySelector(".ic-details-root"), null);
});

test("history rows and user choices open context menus without mute buttons or changing filters", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const calls = [];
  const user = { id: "9007199254740993", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {}, { open: (event, ...args) => calls.push({ event, args }) });
  try {
    menu.update({ channel, participants: { [user.id]: user }, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "joined" }] });
    assert.equal(document.querySelector('[data-mute-id]'), null);
    const row = document.querySelector(".cm-event");
    assert.equal(row.getAttribute("aria-haspopup"), "menu");
    const rightClick = new window.Event("contextmenu", { bubbles: true, cancelable: true });
    row.dispatchEvent(rightClick);
    assert.equal(rightClick.defaultPrevented, true);
    assert.deepEqual(calls[0].args, [user.id, channel.id]);
    const keyboard = new window.Event("keydown", { bubbles: true, cancelable: true });
    keyboard.key = "F10";
    keyboard.shiftKey = true;
    row.dispatchEvent(keyboard);
    assert.deepEqual(calls[1].args, [user.id, channel.id]);
    document.querySelector(".cm-user-option").dispatchEvent(new window.Event("contextmenu", { bubbles: true, cancelable: true }));
    assert.deepEqual(calls[2].args, [user.id, undefined]);
    assert.equal(document.querySelector('[data-user-id]').checked, true);
    assert.equal(document.querySelectorAll(".cm-event").length, 1);
  } finally { menu.dispose(); }
});

test("export downloads only matching events as a Markdown file", async () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;
  let blob;
  let filename;
  URL.createObjectURL = value => { blob = value; return "blob:markdown-test"; };
  URL.revokeObjectURL = () => {};
  document.addEventListener("click", event => {
    if (event.target.tagName === "A") { filename = event.target.download; event.preventDefault(); }
  });
  const user = { id: "9007199254740993", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {});
  try {
    const download = document.querySelector('[aria-label="Export matching activity as Markdown"]');
    assert.equal(download.disabled, true);
    menu.update({ channel, participants: {}, events: [
      { sequence: 1, user, channel, timestamp: 1000, action: "joined" },
      { sequence: 2, user: { ...user, id: "2", username: "other", display_name: "Other" }, channel, timestamp: 2000, action: "left" },
    ] });
    const search = document.querySelector('[aria-label="Search history"]');
    search.value = "morgan";
    search.dispatchEvent(new window.Event("input"));
    download.click();
    assert.match(filename, /^callmon-voice-activity-.*\.md$/);
    assert.equal(blob.type, "text/markdown;charset=utf-8");
    const markdown = await blob.text();
    assert.ok(markdown.startsWith("# Callmon Voice Activity\n"));
    assert.ok(markdown.includes("Events: 1"));
    assert.ok(markdown.includes(user.id));
    assert.ok(!markdown.includes("Other"));
    search.value = "no matching user";
    search.dispatchEvent(new window.Event("input"));
    assert.equal(download.disabled, true);
  } finally {
    menu.dispose();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  }
});

test("archives are browsable, read-only, and isolated from the current call", async () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const sent = [];
  const opened = [];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;
  let blob;
  URL.createObjectURL = value => { blob = value; return "blob:archive-test"; };
  URL.revokeObjectURL = () => {};
  const oldUser = { id: "9007199254740993", username: "old_user", display_name: "Old User" };
  const currentUser = { id: "2", username: "current_user", display_name: "Current User" };
  const oldChannel = { id: "100", name: "Old call" };
  const channel = { id: "200", name: "New call" };
  const archive = { id: 1, channel: oldChannel, started_at: 1000, ended_at: 3000,
    events: [{ sequence: 1, user: oldUser, channel: oldChannel, timestamp: 2000, action: "joined" }] };
  const state = { channel, current_call: { id: 2, channel, started_at: 4000, ended_at: null },
    participants: { [currentUser.id]: currentUser }, events: [], archives: [archive] };
  const menu = mountMenu(payload => sent.push(payload), { open: (event, ...args) => opened.push(args) });
  try {
    menu.update(state);
    const picker = document.querySelector('[aria-label="Call history"]');
    assert.equal(picker.hidden, false);
    assert.equal(document.querySelectorAll(".cm-event").length, 0);
    assert.equal(document.querySelectorAll(".cm-user-option").length, 1);
    function select(id) {
      for (const option of picker.children) option.selected = option.value === id;
      picker.dispatchEvent(new window.Event("change"));
    }
    select("1");
    assert.match(document.querySelector(".cm-session").textContent, /Archived call/);
    assert.equal(document.querySelector(".cm-event .cm-name").textContent, "Old User");
    assert.equal(document.querySelectorAll(".cm-user-option").length, 1);
    document.querySelector(".cm-user-option").dispatchEvent(new window.Event("contextmenu", { cancelable: true }));
    assert.deepEqual(opened[0], [oldUser.id, oldChannel.id]);
    const clear = document.querySelector('[aria-label="Clear history"]');
    assert.equal(clear.disabled, true);
    clear.click();
    clear.click();
    assert.deepEqual(sent, []);
    document.querySelector('[aria-label="Export matching activity as Markdown"]').click();
    const markdown = await blob.text();
    assert.ok(markdown.includes("Call: Old call (100)"));
    assert.ok(markdown.includes("Ended: 1970-01-01T00:00:03.000Z"));
    assert.ok(markdown.includes(oldUser.id));
    assert.ok(!markdown.includes("Current User"));
    menu.update({ ...state, events: [{ sequence: 2, user: currentUser, channel, timestamp: 5000, action: "joined" }] });
    assert.equal(document.querySelector(".cm-event .cm-name").textContent, "Old User");
    menu.update({ ...state, current_call: { ...state.current_call, id: 3 }, archives: [archive] });
    assert.match(document.querySelector(".cm-session").textContent, /This call/);
    assert.equal(document.querySelectorAll(".cm-event").length, 0);
    select("1");
    menu.update({ ...state, current_call: { ...state.current_call, id: 3 }, archives: [] });
    assert.equal(picker.hidden, true);
    assert.match(document.querySelector(".cm-session").textContent, /This call/);
  } finally {
    menu.dispose();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  }
});

test("unsupported native context menus fail visibly while leaving history usable", () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {});
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "left" }] });
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { cancelable: true }));
    assert.match(document.querySelector(".cm-status").textContent, /context menu unavailable/);
    assert.equal(document.querySelectorAll(".cm-event").length, 1);
  } finally { menu.dispose(); }
});

test("asynchronous native menu failures are visible and leave history usable", async () => {
  const { window, document } = parseHTML("<html><body></body></html>");
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  const menu = mountMenu(() => {}, { open: async () => { throw new Error("Chunk download failed"); } });
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "left" }] });
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { cancelable: true }));
    await Promise.resolve();
    assert.equal(document.querySelector(".cm-status").textContent, "Chunk download failed");
    assert.equal(document.querySelectorAll(".cm-event").length, 1);
  } finally { menu.dispose(); }
});

test("lazy native menus protect history during loading and after early dismissal", async () => {
  const { window, document } = parseHTML('<html><body><button id="outside">Outside</button></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  let finishOpening;
  const pending = new Promise(resolve => { finishOpening = resolve; });
  const menu = mountMenu(() => {}, { open: () => pending });
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "joined" }] });
    document.querySelector(".cm-launcher").click();
    const panel = document.querySelector(".cm-panel");
    const outside = document.querySelector("#outside");
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { bubbles: true, cancelable: true }));
    outside.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, false);
    outside.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, false);
    finishOpening();
    await pending;
    await Promise.resolve();
    outside.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, false);
    outside.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, true);
  } finally { finishOpening(); menu.dispose(); }
});

test("native user menu interactions and dismissal keep Callmon open", () => {
  const { window, document } = parseHTML('<html><body><button id="outside">Outside</button></body></html>');
  globalThis.window = window;
  globalThis.document = document;
  const user = { id: "1", username: "morgan", display_name: "Morgan" };
  const channel = { id: "100", name: "General" };
  let nativeMenu;
  const menu = mountMenu(() => {}, { open() {
    nativeMenu = document.createElement("div");
    nativeMenu.setAttribute("role", "menu");
    const action = document.createElement("button");
    action.textContent = "Mute";
    nativeMenu.append(action);
    document.body.append(nativeMenu);
  } });
  try {
    menu.update({ channel, participants: {}, events: [{ sequence: 1, user, channel, timestamp: 1000, action: "joined" }] });
    document.querySelector(".cm-launcher").click();
    const panel = document.querySelector(".cm-panel");
    document.querySelector(".cm-event").dispatchEvent(new window.Event("contextmenu", { bubbles: true, cancelable: true }));
    assert.equal(panel.hidden, false);
    nativeMenu.firstElementChild.dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, false);
    const escape = new window.Event("keydown", { bubbles: true });
    escape.key = "Escape";
    nativeMenu.dispatchEvent(escape);
    assert.equal(panel.hidden, false);
    document.querySelector("#outside").dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    nativeMenu.remove();
    assert.equal(panel.hidden, false);
    document.querySelector("#outside").dispatchEvent(new window.Event("pointerdown", { bubbles: true }));
    assert.equal(panel.hidden, true);
  } finally { nativeMenu?.remove(); menu.dispose(); }
});