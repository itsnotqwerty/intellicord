import test from "node:test";
import assert from "node:assert/strict";
import { createVoiceWatch } from "../src/web/watch.js";

const first = "9007199254740993";
const second = "9007199254740992";
const channel = "9007199254740995";

test("explicit watch state is idempotent and listing returns an isolated exact-ID snapshot", () => {
  const state = fixture([first]);
  const watch = createVoiceWatch(state.stores);
  let notifications = 0;
  watch.subscribe(() => notifications++);
  try {
    assert.equal(watch.setWatched(first, true), true);
    assert.equal(watch.setWatched(first, true), true);
    assert.equal(notifications, 1);
    const listed = watch.listWatched();
    assert.deepEqual(listed, [first]);
    listed.push(second);
    assert.deepEqual(watch.listWatched(), [first]);
    state.emit();
    assert.deepEqual(watch.events, []);
    assert.equal(watch.setWatched(first, false), false);
    assert.equal(watch.setWatched(first, false), false);
    assert.equal(notifications, 2);
    assert.deepEqual(watch.listWatched(), []);
    assert.throws(() => watch.setWatched(first, "true"), /boolean/);
    assert.throws(() => watch.setWatched(Number(first), true), /Invalid/);
    for (let index = 1; index <= 100; index++) watch.setWatched(String(index), true);
    assert.equal(watch.setWatched("101", true), false);
    assert.equal(watch.listWatched().length, 100);
  } finally { watch.dispose(); }
  assert.deepEqual(watch.listWatched(), []);
  assert.equal(watch.setWatched(first, true), false);
});

function fixture(initial = [], initialChannel = channel) {
  let selectedId = initialChannel;
  let states = initial.map(userId => ({ userId, channelId: initialChannel }));
  const voiceListeners = new Set();
  const selectedListeners = new Set();
  const reads = [];
  const userReads = [];
  const errors = [];
  const users = new Map([[first, { id: first, username: "morgan", globalName: "Morgan", email: "private" }]]);
  const stores = {
    voice: {
      getVoiceStatesForChannel(id) { reads.push(id); assert.equal(id, selectedId); return states; },
      addChangeListener: listener => voiceListeners.add(listener),
      removeChangeListener: listener => voiceListeners.delete(listener),
    },
    selected: {
      getVoiceChannelId: () => selectedId,
      addChangeListener: listener => selectedListeners.add(listener),
      removeChangeListener: listener => selectedListeners.delete(listener),
    },
    users: { getUser(id) { userReads.push(id); return users.get(id); } },
    onError: error => errors.push(error),
  };
  return { stores, reads, userReads, errors, voiceListeners, selectedListeners,
    emit() { for (const listener of [...voiceListeners]) listener(); },
    set(ids) { states = ids.map(userId => ({ userId, channelId: selectedId })); this.emit(); },
    select(id, ids = []) {
      selectedId = id;
      states = ids.map(userId => ({ userId, channelId: id }));
      for (const listener of [...selectedListeners]) listener();
    },
    raw(value) { states = value; this.emit(); },
  };
}

test("exact IDs, all users including self, and existing local names only", () => {
  const fixtureState = fixture();
  const received = [];
  const watch = createVoiceWatch(fixtureState.stores, event => received.push(event));
  try {
    assert.equal(watch.toggle(first), true);
    assert.equal(watch.toggle(second), true);
    fixtureState.set([first, second, "7"]);
    assert.deepEqual(received.map(event => [event.action, event.user.id]), [["joined", first], ["joined", second]]);
    assert.deepEqual(received[0].user, { id: first, username: "morgan", display_name: "Morgan" });
    assert.deepEqual(received[1].user, { id: second, username: null, display_name: null });
    assert.deepEqual(received[0].channel, { id: channel });
    assert.ok(Number.isSafeInteger(received[0].timestamp));
    assert.deepEqual(fixtureState.userReads, [first, second]);
    fixtureState.set([second]);
    assert.equal(received.at(-1).action, "left");
    assert.equal(received.at(-1).user.id, first);
    fixtureState.emit();
    assert.equal(received.length, 3);
    assert.equal(watch.toggle(second), false);
    fixtureState.set([]);
    assert.equal(received.length, 3);
    assert.throws(() => watch.toggle(Number(first)), /Invalid/);
    assert.equal(watch.isWatched(first), true);
  } finally { watch.dispose(); }
});

test("empty baseline observes the first join; populated baseline does not synthesize joins", () => {
  const populated = fixture([first]);
  const watch = createVoiceWatch(populated.stores);
  try {
    watch.toggle(first);
    populated.emit();
    assert.deepEqual(watch.events, []);
    populated.set([]);
    assert.equal(watch.events[0].action, "left");
    populated.set([first]);
    assert.equal(watch.events[1].action, "joined");
  } finally { watch.dispose(); }
});

test("switches and no-channel transitions reset baseline without cross-channel reads", () => {
  const state = fixture([first]);
  const watch = createVoiceWatch(state.stores);
  try {
    watch.toggle(first);
    state.select("200", []);
    assert.equal(watch.events.length, 0);
    state.set([first]);
    assert.equal(watch.events.length, 1);
    assert.equal(watch.events[0].channel.id, "200");
    const reads = state.reads.length;
    state.select(null);
    state.emit();
    assert.equal(state.reads.length, reads);
    assert.equal(watch.events.length, 1);
    state.select(channel, [first]);
    assert.equal(watch.events.length, 1);
    state.set([]);
    assert.equal(watch.events.at(-1).action, "left");
    assert.deepEqual(new Set(state.reads), new Set([channel, "200"]));
  } finally { watch.dispose(); }
});

test("watches, history, subscribers and callback delivery are bounded and isolated", () => {
  const state = fixture();
  let calls = 0;
  const watch = createVoiceWatch(state.stores, () => { calls++; throw new Error("callback failed"); });
  try {
    for (let index = 1; index <= 100; index++) assert.equal(watch.toggle(String(index)), true);
    assert.equal(watch.toggle("101"), false);
    assert.equal(watch.isWatched("101"), false);
    assert.match(state.errors[0].message, /limit/);
    let notifications = 0;
    const unsubscribe = watch.subscribe(() => notifications++);
    watch.subscribe(() => { throw new Error("subscriber failed"); });
    for (let index = 0; index < 102; index++) state.set(index % 2 === 0 ? ["1"] : []);
    assert.equal(calls, 102);
    assert.equal(notifications, 102);
    assert.equal(watch.events.length, 100);
    assert.equal(watch.events[0].sequence, 3);
    const events = watch.events;
    events.length = 0;
    assert.equal(watch.events.length, 100);
    assert.throws(() => { watch.events[0].user.id = "2"; }, TypeError);
    unsubscribe();
    state.set(["1"]);
    assert.equal(notifications, 102);
    for (let index = 0; index < 99; index++) watch.subscribe(() => {});
    assert.throws(() => watch.subscribe(() => {}), /limit/);
    assert.ok(state.errors.some(error => error.message === "callback failed"));
    assert.ok(state.errors.some(error => error.message === "subscriber failed"));
    assert.equal(watch.toggle("100"), false);
    assert.equal(watch.toggle("101"), true);
  } finally { watch.dispose(); }
});

test("unsupported states fail locally and recovery rebaselines; other-channel and duplicate states do not emit", () => {
  const state = fixture();
  const watch = createVoiceWatch(state.stores);
  try {
    watch.toggle(first);
    state.raw([{ userId: first, channelId: "elsewhere" }]);
    assert.equal(watch.events.length, 0);
    state.raw([{ userId: Number(first) }]);
    assert.match(state.errors.at(-1).message, /user ID/);
    state.set([first]);
    assert.equal(watch.events.length, 0);
    state.raw([{ userId: first }, { userId: first }]);
    assert.equal(watch.events.length, 0);
    state.set([]);
    assert.equal(watch.events.length, 1);
    state.raw(null);
    assert.match(state.errors.at(-1).message, /state format/);
  } finally { watch.dispose(); }
});

test("dispose removes both listeners, clears memory, and makes saved callbacks inert", () => {
  const state = fixture();
  const watch = createVoiceWatch(state.stores);
  const saved = [...state.voiceListeners][0];
  watch.toggle(first);
  state.set([first]);
  watch.dispose();
  watch.dispose();
  assert.equal(state.voiceListeners.size, 0);
  assert.equal(state.selectedListeners.size, 0);
  assert.equal(watch.isWatched(first), false);
  assert.deepEqual(watch.events, []);
  assert.equal(watch.toggle(first), false);
  saved();
  state.set([]);
  assert.deepEqual(watch.events, []);
  watch.subscribe(() => assert.fail("Disposed callback"))();
});

test("partial listener registration rolls back without leaving a voice observer", () => {
  const state = fixture();
  state.stores.selected.addChangeListener = () => { throw new Error("Registration failed"); };
  assert.throws(() => createVoiceWatch(state.stores), /Registration failed/);
  assert.equal(state.voiceListeners.size, 0);
  assert.equal(state.selectedListeners.size, 0);
});

test("reentrant subscribers and error handlers cannot recursively deliver unbounded callbacks", () => {
  const state = fixture();
  let errors = 0;
  let calls = 0;
  let watch;
  watch = createVoiceWatch(state.stores, null, () => {
    errors++;
    watch.toggle("101");
  });
  try {
    for (let index = 1; index <= 100; index++) watch.toggle(String(index));
    watch.subscribe(() => {
      calls++;
      watch.toggle(first);
      watch.toggle("1");
    });
    watch.toggle("101");
    assert.equal(errors, 1);
    watch.toggle("1");
    assert.equal(calls, 1);
    assert.equal(errors, 2);
  } finally { watch.dispose(); }
});

test("event callbacks can unwatch or dispose before later users are delivered", () => {
  const state = fixture();
  const received = [];
  let watch;
  watch = createVoiceWatch(state.stores, event => {
    received.push(event);
    if (event.user.id === first) watch.toggle(second);
    else watch.dispose();
    state.emit();
  });
  try {
    watch.toggle(first);
    watch.toggle(second);
    state.set([first, second]);
    assert.deepEqual(received.map(event => event.user.id), [first]);
    assert.equal(watch.events.length, 1);
    watch.toggle(second);
    state.set([]);
    assert.deepEqual(received.map(event => event.user.id), [first, first]);
    watch.toggle(first);
    watch.toggle(second);
    state.set([first, second]);
    assert.equal(received.at(-1).user.id, second);
    assert.deepEqual(watch.events, []);
  } finally { watch.dispose(); }
});