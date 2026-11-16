import test from "node:test";
import assert from "node:assert/strict";
import { createLocalMute, createUserContextMenu, discoverClientStores, discoverStores, observeCall } from "../src/web/bridge.js";

test("user context menus delegate exact users and channels to Discord's native component", () => {
  const { stores } = fixture();
  const user = stores.users.getUser("9007199254740993");
  const channel = { id: "100", getGuildId: () => "200" };
  stores.users.getUser = id => id === user.id ? user : null;
  stores.channels.getChannel = id => id === channel.id ? channel : null;
  const component = () => {};
  let opened;
  const opener = (event, render) => { opened = { event, element: render({ context: "APP" }) }; };
  opener.toString = () => "currentTarget.contains renderLazy: preventDefault";
  const runtime = id => {
    assert.equal(id, "native-user-menu");
    return { [Symbol("rspack exports")]: { default: component } };
  };
  runtime.m = { "native-user-menu": { toString: () => 'navId:"user-context" GuildChannelUserContextMenu' } };
  runtime.c = { opener: { exports: { open: opener } }, jsx: { exports: { jsx: (type, props) => ({ type, props }) } } };
  stores.runtime = runtime;
  const menu = createUserContextMenu(stores);
  const event = {};
  menu.open(event, user.id, channel.id);
  assert.equal(opened.event, event);
  assert.equal(opened.element.type, component);
  assert.equal(opened.element.props.user, user);
  assert.equal(opened.element.props.channel, channel);
  assert.equal(opened.element.props.guildId, "200");
  assert.equal(opened.element.props.context, "APP");
  assert.equal(opened.element.props.showMediaItems, true);
  assert.equal(opened.element.props.showChannelCallItems, true);
  menu.open(event, user.id);
  assert.equal(opened.element.props.channel, channel);
  assert.throws(() => menu.open(event, "missing", channel.id), /no longer available/);
  assert.throws(() => menu.open(event, user.id, "deleted-channel"), /no longer available/);
  runtime.m = {};
  assert.throws(() => menu.open(event, user.id, channel.id), /unavailable on this build/);
});

test("cold-cache native menus load only the chunks from the matching Discord call site", async () => {
  const { stores } = fixture();
  const component = () => {};
  let opened;
  const opener = (event, render) => { opened = { event, element: render({ context: "APP" }) }; };
  opener.toString = () => "currentTarget.contains renderLazy: preventDefault";
  const loaded = [];
  const runtime = id => {
    assert.equal(id, 107632);
    assert.deepEqual(loaded, [339715, 24312]);
    return { [Symbol("rspack exports")]: { default: component } };
  };
  runtime.m = { 279154(native) {
    return async () => {
      const { default: menu } = await Promise.all([native.e(339715), native.e(24312)]).then(native.bind(native, 107632));
      return { menu, onInteraction: "GuildChannelUserContextMenu" };
    };
  } };
  runtime.c = { opener: { exports: { open: opener } }, jsx: { exports: { jsx: (type, props) => ({ type, props }) } } };
  runtime.e = async id => { loaded.push(id); };
  stores.runtime = runtime;
  stores.channels.getChannel = id => ({ id, guild_id: "200" });
  const target = {};
  const event = { target, currentTarget: target, pageX: 12, pageY: 34, preventDefault() {}, stopPropagation() {} };
  const pending = createUserContextMenu(stores).open(event, "9007199254740993", "100");
  event.currentTarget = null;
  await pending;
  assert.equal(opened.event.currentTarget, target);
  assert.equal(opened.event.pageX, 12);
  assert.equal(opened.element.type, component);
  assert.equal(opened.element.props.guildId, "200");
  assert.equal(opened.element.props.user.id, "9007199254740993");
  loaded.length = 0;
  await createUserContextMenu(stores).open({ target, currentTarget: target }, "9007199254740993", "100");
  assert.equal(opened.event.pageX, 0);
  assert.equal(opened.event.pageY, 0);
  stores.channels.getChannel = id => ({ id });
  runtime.m.genericPanel = function(native) {
    return async () => {
      const { default: menu } = await Promise.all([native.e(924697)]).then(native.bind(native, 668569));
      return { menu, onInteraction: "UserGenericContextMenu" };
    };
  };
  const genericRuntime = id => {
    assert.equal(id, 668569);
    return { default: component };
  };
  Object.assign(genericRuntime, runtime);
  loaded.length = 0;
  stores.runtime = genericRuntime;
  await createUserContextMenu(stores).open(event, "9007199254740993", "100");
  assert.deepEqual(loaded, [924697]);
  assert.equal(opened.element.props.guildId, undefined);
  stores.runtime = runtime;
  stores.channels.getChannel = id => ({ id, guild_id: "200" });
  runtime.e = async () => { throw new Error("Chunk download failed"); };
  await assert.rejects(createUserContextMenu(stores).open(event, "9007199254740993", "100"), /Chunk download failed/);
  runtime.m = { unrelated: function(native) {
    return async () => {
      const { default: menu } = await Promise.all([native.e("unknown")]).then(native.bind(native, 107632));
      return { menu, onInteraction: "GuildChannelUserContextMenu" };
    };
  } };
  assert.throws(() => createUserContextMenu(stores).open(event, "9007199254740993", "100"), /unavailable on this build/);
});

function fixture() {
  let channelId = "100";
  let states = { one: { userId: "9007199254740993" } };
  const listeners = new Set();
  const store = methods => ({ ...methods,
    addChangeListener: callback => listeners.add(callback),
    removeChangeListener: callback => listeners.delete(callback),
  });
  const stores = {
    voice: store({ getVoiceStatesForChannel: () => states }),
    selected: store({ getVoiceChannelId: () => channelId }),
    users: { getUser: id => ({ id, username: "morgan", globalName: "Morgan" }) },
    channels: { getChannel: id => ({ id, name: "General" }) },
  };
  return { stores, listeners, setChannel: id => { channelId = id; }, setStates: value => { states = value; } };
}

test("discovery supports nested exports and fails closed on missing stores", () => {
  const { stores } = fixture();
  const require = { c: Object.fromEntries(Object.values(stores).map((store, index) => [index, { exports: { default: store } }])) };
  assert.deepEqual(discoverStores(require), stores);
  assert.throws(() => discoverStores({ c: {} }), /required/);
  assert.throws(() => discoverStores({}), /cache/);
});

test("snapshots use native profile pictures and safely retain hash-based fallbacks", () => {
  const { stores, listeners } = fixture();
  const snapshots = [];
  let nativeUrl = "https://cdn.discordapp.com/avatars/9007199254740993/a_abcdef.gif?size=64";
  stores.users.getUser = id => ({ id, username: "morgan", globalName: "Morgan", avatar: "a_abcdef", getAvatarURL: (...args) => {
    assert.deepEqual(args, [null, 64, true]);
    return nativeUrl;
  } });
  const stop = observeCall(stores, payload => snapshots.push(JSON.parse(payload)), () => {});
  assert.equal(snapshots[0].users[0].avatar_url, nativeUrl);
  nativeUrl = "https://tracking.example/avatar.png";
  for (const listener of listeners) listener();
  assert.equal(snapshots.at(-1).users[0].avatar_url, "https://cdn.discordapp.com/avatars/9007199254740993/a_abcdef.gif?size=64");
  stores.users.getUser = id => ({ id, username: "morgan", globalName: "Morgan", getAvatarURL() { throw new Error("unavailable"); } });
  for (const listener of listeners) listener();
  assert.equal(snapshots.at(-1).users[0].avatar_url, null);
  stop();
});

test("discovery unwraps resolved Rspack async module exports", () => {
  const { stores } = fixture();
  const symbol = Symbol("rspack exports");
  const require = { c: Object.fromEntries(Object.values(stores).map((store, index) =>
    [index, { exports: { [symbol]: { default: store } } }])) };
  assert.deepEqual(discoverStores(require), stores);
});

test("runtime discovery keeps the app runtime instead of the last shared runtime", () => {
  const { stores } = fixture();
  const require = { c: Object.fromEntries(Object.values(stores).map((store, index) => [index, { exports: { default: store } }])) };
  const chunks = [];
  chunks.push = chunk => {
    Array.prototype.push.call(chunks, chunk);
    chunk[2](require);
    chunk[2]({ c: {} });
  };
  assert.deepEqual(discoverClientStores(chunks), stores);
  assert.equal(chunks.length, 0);
});

test("discovery ignores translation proxies that synthesize store methods", () => {
  const { stores } = fixture();
  const translation = new Proxy({}, { get: () => () => ({ locale: "en-US", ast: [] }) });
  const require = { c: { translation: { exports: { default: translation } },
    ...Object.fromEntries(Object.values(stores).map((store, index) => [`store_${index}`, { exports: { default: store } }])) } };
  assert.deepEqual(discoverStores(require), stores);
});

test("discovery prefers named stores over helpers with overlapping methods", () => {
  const { stores } = fixture();
  const names = { voice: "VoiceStateStore", selected: "SelectedChannelStore", users: "UserStore", channels: "ChannelStore" };
  for (const [key, store] of Object.entries(stores)) store.getName = () => names[key];
  const helper = { getChannel: () => null };
  const require = { c: { helper: { exports: { default: helper } },
    ...Object.fromEntries(Object.values(stores).map((store, index) => [`store_${index}`, { exports: { default: store } }])) } };
  assert.deepEqual(discoverStores(require), stores);
});

test("local mute uses the native action and refreshes from the media store", () => {
  const { stores } = fixture();
  const calls = [];
  const listeners = new Set();
  let muted = false;
  stores.users.getCurrentUser = () => ({ id: "1" });
  stores.media = { isLocalMute: () => muted,
    addChangeListener: listener => listeners.add(listener), removeChangeListener: listener => listeners.delete(listener) };
  stores.muteActions = { toggleLocalMute: (...args) => { calls.push(args); muted = !muted; } };
  const mute = createLocalMute(stores);
  assert.equal(mute.unavailableReason("9007199254740993"), null);
  mute.toggle("9007199254740993");
  assert.deepEqual(calls, [["9007199254740993"]]);
  assert.equal(mute.isMuted("9007199254740993"), true);
  let updates = 0;
  const stop = mute.subscribe(() => { updates++; });
  for (const listener of listeners) listener();
  assert.equal(updates, 1);
  stop();
  assert.equal(listeners.size, 0);
});

test("local mute refuses self, absent users, and missing identity or APIs", () => {
  const { stores } = fixture();
  const calls = [];
  stores.users.getCurrentUser = () => ({ id: "1" });
  stores.media = { isLocalMute: () => false };
  stores.muteActions = { toggleLocalMute: id => calls.push(id) };
  const mute = createLocalMute(stores);
  assert.throws(() => mute.toggle("1"), /yourself/);
  assert.throws(() => mute.toggle("2"), /not in the current call/);
  stores.users.getCurrentUser = () => null;
  assert.throws(() => mute.toggle("9007199254740993"), /Current user unavailable/);
  assert.equal(calls.length, 0);
  assert.equal(createLocalMute(fixture().stores), null);
});

test("snapshots keep exact IDs, suppress duplicates, follow current call, and detach", () => {
  const fixtureData = fixture();
  const messages = [];
  const stop = observeCall(fixtureData.stores, payload => messages.push(JSON.parse(payload)), () => {});
  try {
    assert.equal(messages[0].users[0].id, "9007199254740993");
    for (const listener of fixtureData.listeners) listener();
    assert.equal(messages.length, 1);
    fixtureData.setStates({});
    for (const listener of fixtureData.listeners) listener();
    assert.deepEqual(messages[1].users, []);
    fixtureData.setChannel(null);
    for (const listener of fixtureData.listeners) listener();
    assert.equal(messages[2].channel, null);
  } finally { stop(); }
  assert.equal(fixtureData.listeners.size, 0);
});

test("malformed or unresolved participants pause capture without false leave snapshots", () => {
  const fixtureData = fixture();
  const messages = [];
  const errors = [];
  const stop = observeCall(fixtureData.stores, payload => messages.push(payload), error => errors.push(error));
  try {
    fixtureData.setStates({ bad: { userId: 9007199254740993 } });
    for (const listener of fixtureData.listeners) listener();
    assert.equal(messages.length, 1);
    assert.match(errors.at(-1), /ID format/);
    fixtureData.setStates({ one: { userId: "2" } });
    fixtureData.stores.users.getUser = () => null;
    for (const listener of fixtureData.listeners) listener();
    assert.equal(messages.length, 1);
    assert.match(errors.at(-1), /not yet available/);
  } finally { stop(); }
});