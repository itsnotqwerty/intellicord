import test from "node:test";
import assert from "node:assert/strict";
import { createObliterateAdapter, obliterateMenuContext } from "../src/web/bridge.js";

test("native menu target resolution requires one exact user/channel pair and terminates on cycles", () => {
  const menu = { id: "user-context", __reactFiber$test: { memoizedProps: { user: { id: "1" }, channel: { id: "100" } } } };
  assert.deepEqual(obliterateMenuContext(menu), { userId: "1", channelId: "100" });
  menu.__reactFiber$test.return = { memoizedProps: { userId: "2", channelId: "200" } };
  assert.equal(obliterateMenuContext(menu), null);
  menu.__reactFiber$test.return = menu.__reactFiber$test;
  assert.deepEqual(obliterateMenuContext(menu), { userId: "1", channelId: "100" });
  assert.equal(obliterateMenuContext({ id: "message-context" }), null);
  assert.equal(obliterateMenuContext({ id: "user-context" }), null);
  menu.__reactFiber$test.memoizedProps = { userId: 1, channelId: "100" };
  assert.equal(obliterateMenuContext(menu), null);
});

function fixture() {
  const calls = [];
  const state = { mute: false, soundboard: false, video: false, blocked: false, ignored: false, ban: true, kick: true, moderate: false, timeout: true };
  const context = { userId: "9007199254740993", channelId: "100" };
  const target = { id: context.userId, username: "Target", bot: false, isProvisional: false, isNonUserBot: () => false };
  const named = (name, methods) => ({ getName: () => name, ...methods });
  const actions = Object.fromEntries([
    ["toggleLocalMute", "mute"], ["toggleLocalSoundboardMute", "soundboard"], ["setDisableLocalVideo", "video"],
    ["ignoreUser", "ignored"], ["blockUser", "blocked"], ["banUser", null], ["kickUser", null], ["setCommunicationDisabledUntil", null],
  ].map(([key, field]) => [key, (...args) => { calls.push([key, ...args]); if (field) state[field] = true; }]));
  actions.setCommunicationDisabledUntil = function setCommunicationDisabledUntil({ guildId, userId, communicationDisabledUntilTimestamp, duration, reason, location, moderatorReportId }) {
    calls.push(["setCommunicationDisabledUntil", { guildId, userId, communicationDisabledUntilTimestamp, duration, reason, location, moderatorReportId }]);
  };
  const enums = { BAN_MEMBERS: 4n, KICK_MEMBERS: 2n, MODERATE_MEMBERS: 1099511627776n };
  const modes = { DISABLED: 73, MANUAL_ENABLED: 11, AUTO_PROBING: 12, AUTO_ENABLED: 13, NONE: 14 };
  const stores = {
    users: { getCurrentUser: () => ({ id: "1" }), getUser: userId => userId === "1" ? { id: "1" } : target },
    selected: { getVoiceChannelId: () => "100" },
    voice: { getVoiceStatesForChannel: () => ({ self: { userId: "1" }, target: { userId: target.id } }) },
    channels: { getChannel: () => ({ guild_id: "200" }) },
    media: named("MediaEngineStore", { isLocalMute: () => state.mute, isLocalVideoDisabled: () => state.video }),
    soundboard: named("SoundboardStore", { isLocalSoundboardMuted: () => state.soundboard }),
    relationships: named("RelationshipStore", { isBlocked: () => state.blocked, isIgnored: () => !state.blocked && state.ignored }),
    permissions: named("PermissionStore", { canManageUser: permission => permission === 4n ? state.ban : permission === 2n ? state.kick : state.moderate }),
    guilds: named("GuildStore", { getGuild: () => ({ id: "200", name: "Server" }) }),
    members: named("GuildMemberStore", { getMember: () => ({}) }),
    runtime: { c: { cached: { exports: { actions, enums, videoConstants: { modes } } } } },
  };
  stores.runtime.c.cached.exports.timeoutPredicate = function timeoutPredicate(guildId, userId) {
    const [userStore, guildStore, permissionStore] = arguments[2];
    const user = userStore.getUser(userId);
    const guild = guildStore.getGuild(guildId);
    return !user.isNonUserBot() && !user.owner && !user.ADMINISTRATOR && state.timeout &&
      permissionStore.canManageUser(enums.MODERATE_MEMBERS, user, guild);
  };
  const adapter = createObliterateAdapter(stores);
  const plan = () => adapter.plan(context);
  const action = key => plan().actions.find(value => value.key === key);
  return { calls, state, context, target, actions, enums, modes, stores, adapter, plan, action };
}

for (const [actionKey, key, severity] of [
  ["banUser", "moderation", "ban"],
  ["kickUser", "moderation", "kick"],
  ["toggleLocalMute", "mute"],
  ["toggleLocalSoundboardMute", "soundboard"],
  ["setDisableLocalVideo", "video"],
  ["ignoreUser", "ignore"],
  ["blockUser", "block"],
]) {
  test(`${actionKey} rejects distinct competing implementations without mutations`, async () => {
    const { state, stores, adapter, context, action, calls } = fixture();
    if (severity === "kick") state.ban = false;
    const expected = { ...action(key), guildId: "200" };
    assert.equal(expected.status, "ready");
    if (severity === "ban") state.kick = false;
    const before = { ...state };
    stores.runtime.c.cached.exports.competingActions = {
      [actionKey](...args) { calls.push(["competing", ...args]); },
    };
    assert.equal(action(key).status, "unavailable");
    await assert.rejects(adapter.executeAction(context, key, expected));
    assert.deepEqual(calls, []);
    assert.deepEqual(state, before);
  });

  test(`${actionKey} accepts shared-function aliases and preserves the first owner`, async () => {
    const { state, stores, actions, adapter, context, action, calls } = fixture();
    if (severity === "kick") state.ban = false;
    const original = actions[actionKey];
    actions[actionKey] = function (...args) {
      assert.equal(this, actions);
      return original(...args);
    };
    stores.runtime.c.cached.exports.aliasActions = { [actionKey]: actions[actionKey] };
    const expected = { ...action(key), guildId: "200" };
    assert.equal(expected.status, "ready");
    if (severity) assert.equal(expected.severity, severity);
    assert.deepEqual(calls, []);
    assert.equal((await adapter.executeAction(context, key, expected)).status, severity ? "submitted" : "done");
    assert.equal(calls.length, 1);
    assert.equal(calls[0][0], actionKey);
  });
}

test("plan is read-only and returns explicit target, context and six actions", () => {
  const fixtureState = fixture();
  assert.deepEqual(fixtureState.plan().target, { id: fixtureState.target.id, name: "Target" });
  assert.deepEqual(fixtureState.plan().context, { channelId: "100", guildId: "200", guildName: "Server" });
  assert.equal(fixtureState.plan().actions.length, 6);
  assert.ok(fixtureState.plan().actions.every(action => action.status === "ready"));
  assert.deepEqual(fixtureState.calls, []);
});

test("unrelated throwing cached exports cannot invalidate native action discovery", () => {
  for (const candidate of [
    { getName() { throw new Error("Unrelated name lookup"); } },
    Object.defineProperty({}, "getName", { get() { throw new Error("Unrelated name getter"); } }),
    Object.defineProperty({}, "toggleLocalMute", { get() { throw new Error("Unrelated action getter"); } }),
  ]) {
    const { stores, plan, calls } = fixture();
    stores.runtime.c.unrelated = { exports: { candidate } };
    assert.ok(plan().actions.every(action => action.status === "ready"));
    assert.deepEqual(calls, []);
  }
});

test("self actions are all unavailable", async () => {
  const { adapter, calls } = fixture();
  const context = { userId: "1", channelId: "100" };
  assert.ok(adapter.plan(context).actions.every(action => action.status === "unavailable"));
  assert.match(adapter.plan(context).actions[0].reason, /Self/);
  await assert.rejects(adapter.executeAction(context, "block"));
  assert.deepEqual(calls, []);
});

test("missing cached APIs fail closed without loading modules", () => {
  const { stores, plan, calls } = fixture();
  stores.runtime = Object.assign(() => { throw new Error("must not load"); }, { c: {} });
  assert.ok(plan().actions.every(action => action.status === "unavailable"));
  assert.deepEqual(calls, []);
});

test("stale selected context and absent current-call membership prohibit local changes", async () => {
  const { stores, action, adapter, context, calls } = fixture();
  stores.selected.getVoiceChannelId = () => "999";
  assert.equal(action("mute").status, "unavailable");
  await assert.rejects(adapter.executeAction(context, "mute"));
  stores.selected.getVoiceChannelId = () => "100";
  stores.voice.getVoiceStatesForChannel = () => ({ target: { userId: context.userId } });
  assert.equal(action("mute").status, "unavailable");
  assert.deepEqual(calls, []);
});

test("completed mute never toggles back and repeated execution is idempotent", async () => {
  const { adapter, context, calls, state } = fixture();
  assert.equal((await adapter.executeAction(context, "mute")).status, "done");
  assert.equal((await adapter.executeAction(context, "mute")).status, "done");
  assert.equal(state.mute, true);
  assert.deepEqual(calls, [["toggleLocalMute", context.userId, "default"]]);
});

test("local state must be an exact boolean", () => {
  const { state, action } = fixture();
  for (const value of [undefined, null, 0, 1, "false"]) {
    state.mute = value;
    assert.equal(action("mute").status, "unavailable");
  }
});

test("video uses discovered mode value and verified native arguments", async () => {
  const { adapter, context, calls, modes } = fixture();
  assert.equal((await adapter.executeAction(context, "video")).status, "done");
  assert.deepEqual(calls, [["setDisableLocalVideo", context.userId, modes.DISABLED, "default", true, false]]);
});

test("missing video enum cannot guess a disabled value", () => {
  const { modes, action } = fixture();
  delete modes.NONE;
  assert.equal(action("video").status, "unavailable");
});

test("soundboard uses its own store and native default context", async () => {
  const { adapter, context, calls } = fixture();
  assert.equal((await adapter.executeAction(context, "soundboard")).status, "done");
  assert.deepEqual(calls, [["toggleLocalSoundboardMute", context.userId, "default"]]);
});

test("blocking masks ignore and cannot cause an ignore loop", async () => {
  const { adapter, context, action, calls } = fixture();
  assert.equal((await adapter.executeAction(context, "ignore")).status, "done");
  assert.equal(action("ignore").status, "done");
  assert.equal((await adapter.executeAction(context, "block")).status, "done");
  assert.equal(action("ignore").status, "unavailable");
  assert.match(action("ignore").reason, /mask/);
  await assert.rejects(adapter.executeAction(context, "ignore"));
  assert.equal((await adapter.executeAction(context, "block")).status, "done");
  assert.equal(calls.length, 2);
});

test("fulfilled ignore with no store change is unverified", async () => {
  const { actions, adapter, context } = fixture();
  actions.ignoreUser = async () => undefined;
  assert.equal((await adapter.executeAction(context, "ignore")).status, "unverified");
});

test("relationship calls leave unknown native contexts unset and read ignore before blocking", async () => {
  const { stores, actions, adapter, context, calls } = fixture();
  await adapter.executeAction(context, "ignore");
  const order = [];
  const getter = stores.relationships.isIgnored;
  stores.relationships.isIgnored = userId => { order.push("read-ignore"); return getter(userId); };
  const block = actions.blockUser;
  actions.blockUser = (...args) => { order.push("block"); return block(...args); };
  await adapter.executeAction(context, "block");
  assert.deepEqual(order, ["read-ignore", "block"]);
  assert.deepEqual(calls, [
    ["ignoreUser", context.userId, undefined, undefined],
    ["blockUser", context.userId, undefined, undefined],
  ]);
});

test("execution rechecks call membership and requires named native stores", async () => {
  const { stores, adapter, context, action, calls } = fixture();
  assert.equal(action("mute").status, "ready");
  stores.voice.getVoiceStatesForChannel = () => ({ self: { userId: "1" } });
  await assert.rejects(adapter.executeAction(context, "mute"));
  stores.media.getName = () => "UnverifiedStore";
  stores.voice.getVoiceStatesForChannel = () => ({ self: { userId: "1" }, target: { userId: context.userId } });
  assert.equal(action("mute").status, "unavailable");
  assert.deepEqual(calls, []);
});

test("unknown relationship state cannot trigger block or ignore", async () => {
  const { stores, adapter, context, action, calls } = fixture();
  stores.relationships.isBlocked = () => undefined;
  assert.equal(action("ignore").status, "unavailable");
  await assert.rejects(adapter.executeAction(context, "block"));
  stores.relationships.isBlocked = () => false;
  stores.relationships.isIgnored = () => "false";
  assert.equal(action("ignore").status, "unavailable");
  assert.equal(action("block").status, "unavailable");
  assert.deepEqual(calls, []);
});

test("late snapshot failures invalidate earlier ready and done actions before execution", async () => {
  for (const completed of [false, true]) {
    const { state, stores, adapter, context, plan, calls } = fixture();
    state.mute = completed;
    stores.relationships.isIgnored = () => { throw new Error("Relationship state changed"); };
    assert.ok(plan().actions.every(action => action.status === "unavailable"));
    await assert.rejects(adapter.executeAction(context, "mute"), /Native state unavailable/);
    await assert.rejects(adapter.executeAction(context, "block"), /Native state unavailable/);
    assert.deepEqual(calls, []);
  }
});

test("lagging media store and native rejections are not reported as success", async () => {
  const { actions, adapter, context } = fixture();
  actions.toggleLocalMute = () => undefined;
  assert.equal((await adapter.executeAction(context, "mute")).status, "unverified");
  actions.toggleLocalMute = async () => { throw new Error("denied"); };
  await assert.rejects(adapter.executeAction(context, "mute"), /denied/);
});

test("moderation respects native hierarchy, preferring ban then kick", () => {
  const { state, action } = fixture();
  assert.equal(action("moderation").severity, "ban");
  state.ban = false;
  assert.equal(action("moderation").severity, "kick");
  state.kick = false;
  assert.equal(action("moderation").status, "unavailable");
  assert.match(action("moderation").reason, /unavailable|eligible/i);
});

test("permission constant discovery skips protected objects and getters without losing eligibility", () => {
  const { stores, state, action, calls } = fixture();
  let getterCalls = 0;
  const protectedObject = new Proxy({}, { getOwnPropertyDescriptor() { throw new DOMException("Protected browser object", "SecurityError"); } });
  const accessor = Object.defineProperty({}, "BAN_MEMBERS", { get() { getterCalls++; throw new Error("Do not invoke"); } });
  stores.runtime.c = { protected: { exports: { protectedObject, accessor } }, ...stores.runtime.c };
  assert.equal(action("moderation").severity, "ban");
  state.ban = false;
  assert.equal(action("moderation").severity, "kick");
  Object.assign(state, { kick: false, moderate: true });
  assert.equal(action("moderation").severity, "timeout");
  assert.equal(getterCalls, 0);
  assert.deepEqual(calls, []);
});

test("conflicting permission constants fail closed instead of selecting cache order", () => {
  const { stores, state, action } = fixture();
  stores.runtime.c.cached.exports.conflictingPermissions = { BAN_MEMBERS: 8n, KICK_MEMBERS: 16n, MODERATE_MEMBERS: 32n };
  state.moderate = true;
  assert.equal(action("moderation").status, "unavailable");
});

test("server and moderation failures disable only moderation and preserve verified personal operations", async () => {
  for (const failure of ["server", "permissions", "timeout"]) {
    const { state, stores, adapter, context, plan, calls } = fixture();
    if (failure === "server") stores.channels.getChannel = () => { throw new Error("Server context unavailable"); };
    if (failure === "permissions") stores.permissions.canManageUser = () => { throw new Error("Eligibility unavailable"); };
    if (failure === "timeout") {
      Object.assign(state, { ban: false, kick: false, moderate: true });
      Object.defineProperty(state, "timeout", { get() { throw new Error("Timeout state unavailable"); } });
    }
    const result = plan();
    assert.ok(result.actions.slice(0, 5).every(action => action.status === "ready"), failure);
    const moderation = result.actions.at(-1);
    assert.equal(moderation.status, "unavailable");
    assert.match(moderation.reason, /moderation disabled/);
    assert.equal(moderation.severity, undefined);
    assert.deepEqual(calls, []);
    await assert.rejects(adapter.executeAction(context, "moderation", { key: "moderation", severity: "ban", guildId: "200" }));
    assert.deepEqual(calls, []);
    assert.equal((await adapter.executeAction(context, "mute")).status, "done");
    assert.equal(calls.length, 1);
    assert.equal(calls[0][0], "toggleLocalMute");
  }
});

test("timeout fallback uses native predicate stores and the documented 28-day maximum", async () => {
  const { state, adapter, context, action, calls } = fixture();
  Object.assign(state, { ban: false, kick: false, moderate: true });
  const expected = { ...action("moderation"), guildId: "200" };
  assert.equal(expected.severity, "timeout");
  assert.equal(expected.label, "Timeout for 28 days");
  assert.equal(expected.status, "ready");
  assert.doesNotMatch(expected.reason || "", /unsupported/i);
  assert.deepEqual(calls, []);
  const before = Date.now();
  assert.equal((await adapter.executeAction(context, "moderation", expected)).status, "submitted");
  const after = Date.now();
  const timestamp = calls[0]?.[1]?.communicationDisabledUntilTimestamp;
  assert.equal(new Date(timestamp).toISOString(), timestamp);
  assert.ok(Date.parse(timestamp) >= before + 2419200000);
  assert.ok(Date.parse(timestamp) <= after + 2419200000);
  assert.deepEqual(calls, [["setCommunicationDisabledUntil", {
    guildId: "200", userId: context.userId, communicationDisabledUntilTimestamp: timestamp,
    duration: 2419200, reason: undefined, location: undefined, moderatorReportId: undefined,
  }]]);
});

test("timeout requires an unambiguous known plain native predicate and setter", async () => {
  for (const variant of ["missing", "ambiguous", "unknown", "hook", "setter-missing", "setter-ambiguous"]) {
    const { state, stores, actions, adapter, context, action, calls } = fixture();
    Object.assign(state, { ban: false, kick: false, moderate: true });
    const exports = stores.runtime.c.cached.exports;
    if (variant === "missing") delete exports.timeoutPredicate;
    if (variant === "ambiguous") exports.otherPredicate = function otherPredicate(guildId, userId) {
      const [userStore, guildStore, permissionStore] = arguments[2];
      const user = userStore.getUser(userId);
      return !user.isNonUserBot() && !user.ADMINISTRATOR && permissionStore.canManageUser(exports.enums.MODERATE_MEMBERS, user, guildStore.getGuild(guildId));
    };
    if (variant === "unknown") exports.timeoutPredicate = () => true;
    if (variant === "hook") exports.timeoutPredicate = function hook(guildId, userId) {
      return useState(() => stores.permissions.canManageUser(exports.enums.MODERATE_MEMBERS,
        stores.users.getUser(userId), stores.guilds.getGuild(guildId)) && !target.isNonUserBot() && !target.ADMINISTRATOR);
    };
    if (variant === "setter-missing") delete actions.setCommunicationDisabledUntil;
    if (variant === "setter-ambiguous") exports.otherSetter = { setCommunicationDisabledUntil: actions.setCommunicationDisabledUntil };
    assert.equal(action("moderation").status, "unavailable", variant);
    await assert.rejects(adapter.executeAction(context, "moderation", { key: "moderation", severity: "timeout", guildId: "200" }));
    assert.deepEqual(calls, []);
  }
});

test("timeout discovery ignores translation proxies that claim every setter property exists", () => {
  const { state, stores, action } = fixture();
  Object.assign(state, { ban: false, kick: false, moderate: true });
  stores.runtime.c.cached.exports.translationProxy = new Proxy({}, {
    has: () => true,
    get: () => () => undefined,
    getOwnPropertyDescriptor: () => ({ configurable: true, enumerable: true, value: () => undefined }),
  });
  assert.equal(action("moderation").severity, "timeout");
});

test("timeout refuses false or unknown eligibility, owners, administrators, and changed permissions", async () => {
  for (const variant of ["predicate", "unknown-predicate", "permission", "owner", "ADMINISTRATOR", "unknown-permission"]) {
    const { state, target, adapter, context, action, calls } = fixture();
    Object.assign(state, { ban: false, kick: false, moderate: true });
    const expected = { ...action("moderation"), guildId: "200" };
    assert.equal(expected.severity, "timeout");
    if (variant === "predicate") state.timeout = false;
    if (variant === "unknown-predicate") state.timeout = undefined;
    if (variant === "permission") state.moderate = false;
    if (variant === "unknown-permission") state.moderate = undefined;
    if (["owner", "ADMINISTRATOR"].includes(variant)) target[variant] = true;
    assert.equal(action("moderation").status, "unavailable");
    await assert.rejects(adapter.executeAction(context, "moderation", expected));
    assert.deepEqual(calls, []);
  }
});

test("timeout confirmation cannot substitute for a stronger newly available moderation action", async () => {
  const { state, adapter, context, action, calls } = fixture();
  Object.assign(state, { ban: false, kick: false, moderate: true });
  const timeout = { ...action("moderation"), guildId: "200" };
  state.kick = true;
  await assert.rejects(adapter.executeAction(context, "moderation", timeout), /severity/);
  const kick = { ...action("moderation"), guildId: "200" };
  state.kick = false;
  await assert.rejects(adapter.executeAction(context, "moderation", kick), /severity/);
  await assert.rejects(adapter.executeAction(context, "moderation"), /confirmed/);
  assert.deepEqual(calls, []);
});

test("confirmed ban deletes zero messages and reports submitted, not complete", async () => {
  const { adapter, context, action, calls } = fixture();
  assert.equal((await adapter.executeAction(context, "moderation", { ...action("moderation"), guildId: "200" })).status, "submitted");
  assert.deepEqual(calls, [["banUser", "200", context.userId, 0, undefined, undefined]]);
});

test("moderation requires a valid explicit matching confirmed guild ID", async () => {
  const { adapter, context, action, calls } = fixture();
  const expected = action("moderation");
  await assert.rejects(adapter.executeAction(context, "moderation", expected), /guild/);
  for (const guildId of [undefined, null, 200, "", "0", "0200", "invalid", "1".repeat(21), "300"]) {
    await assert.rejects(adapter.executeAction(context, "moderation", { ...expected, guildId }), /guild/);
  }
  assert.deepEqual(calls, []);
});

test("moderation rejects a channel remapped to another server with the same severity", async () => {
  const { adapter, context, stores, action, calls } = fixture();
  const expected = { ...action("moderation"), guildId: "200" };
  stores.channels.getChannel = () => ({ guild_id: "300" });
  stores.guilds.getGuild = () => ({ id: "300", name: "Other server" });
  assert.equal(adapter.plan(context).context.guildId, "300");
  assert.equal(action("moderation").status, "ready");
  assert.equal(action("moderation").severity, expected.severity);
  await assert.rejects(adapter.executeAction(context, "moderation", expected), /guild/);
  assert.deepEqual(calls, []);
});

test("kick uses verified argument order", async () => {
  const { state, adapter, context, action, calls } = fixture();
  state.ban = false;
  await adapter.executeAction(context, "moderation", { ...action("moderation"), guildId: "200" });
  assert.deepEqual(calls, [["kickUser", "200", context.userId, undefined, undefined]]);
});

test("changed permissions cannot escalate or substitute a confirmed severity", async () => {
  const { state, adapter, context, action, calls } = fixture();
  state.ban = false;
  const expected = { ...action("moderation"), guildId: "200" };
  state.ban = true;
  await assert.rejects(adapter.executeAction(context, "moderation", expected), /severity/);
  await assert.rejects(adapter.executeAction(context, "moderation"), /confirmed/);
  const ban = { ...action("moderation"), guildId: "200" };
  state.ban = false;
  await assert.rejects(adapter.executeAction(context, "moderation", ban), /severity/);
  state.kick = false;
  await assert.rejects(adapter.executeAction(context, "moderation", expected));
  assert.deepEqual(calls, []);
});

test("moderation requires explicit guild channel, guild, and both memberships", () => {
  const { adapter, context, stores, action } = fixture();
  assert.equal(adapter.plan({ userId: context.userId }).actions.at(-1).status, "unavailable");
  stores.channels.getChannel = () => ({});
  assert.equal(action("moderation").status, "unavailable");
  stores.channels.getChannel = () => ({ getGuildId: () => "200" });
  stores.members.getMember = (guildId, userId) => userId === "1" ? null : {};
  assert.equal(action("moderation").status, "unavailable");
  stores.members.getMember = (guildId, userId) => userId === context.userId ? null : {};
  assert.equal(action("moderation").status, "unavailable");
  stores.members.getMember = () => ({});
  stores.guilds.getGuild = () => null;
  assert.equal(action("moderation").status, "unavailable");
});

test("unknown and non-user targets fail moderation eligibility", () => {
  const { target, action } = fixture();
  target.bot = true;
  assert.equal(action("moderation").severity, "kick");
  delete target.bot;
  delete target.isProvisional;
  assert.equal(action("moderation").severity, "ban");
  target.bot = false;
  target.isNonUserBot = () => true;
  assert.equal(action("moderation").status, "unavailable");
  target.isNonUserBot = () => undefined;
  assert.equal(action("moderation").status, "unavailable");
  delete target.isNonUserBot;
  assert.equal(action("moderation").status, "unavailable");
  target.isNonUserBot = () => false;
  target.isProvisional = true;
  assert.equal(action("moderation").status, "unavailable");
});

test("permission enums must be cached bigints, never guessed", () => {
  const { enums, action } = fixture();
  delete enums.BAN_MEMBERS;
  enums.KICK_MEMBERS = 2;
  assert.equal(action("moderation").status, "unavailable");
});

test("cached rspack and webpack symbol exports are unwrapped", () => {
  for (const description of ["rspack exports", "webpack exports"]) {
    const { stores, action } = fixture();
    const exports = stores.runtime.c.cached.exports;
    stores.runtime.c.cached.exports = { [Symbol(description)]: exports };
    assert.equal(action("moderation").status, "ready");
  }
});

test("proxy-generated missing methods do not qualify as native APIs", () => {
  const { stores, action } = fixture();
  stores.runtime.c.cached.exports.actions = new Proxy({}, { get: () => () => undefined });
  assert.equal(action("mute").status, "unavailable");
  assert.equal(action("moderation").status, "unavailable");
});

test("unsafe IDs, disappeared users, unknown actions, and disposal never mutate", async () => {
  const { adapter, context, stores, calls } = fixture();
  for (const userId of [1, "0", "01", "1;alert(1)", "1".repeat(21), null]) {
    assert.ok(adapter.plan({ ...context, userId }).actions.every(action => action.status === "unavailable"));
  }
  assert.ok(adapter.plan({ ...context, channelId: 100 }).actions.every(action => action.status === "unavailable"));
  await assert.rejects(adapter.executeAction(context, "unknown"), /Unknown/);
  stores.users.getUser = () => null;
  await assert.rejects(adapter.executeAction(context, "block"));
  adapter.dispose();
  adapter.dispose();
  assert.ok(adapter.plan(context).actions.every(action => action.status === "unavailable"));
  await assert.rejects(adapter.executeAction(context, "mute"), /disposed/);
  assert.deepEqual(calls, []);
});