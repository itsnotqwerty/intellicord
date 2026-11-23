import test from "node:test";
import assert from "node:assert/strict";
import {parseHTML} from "linkedom";
import {quietSurfaceNodes, resolveUserCall, voiceStatusLabels, nativeLinkMenuAnchor} from "../src/web/bridge.js";

test("call names require current-call membership and an available channel", () => {
  const stores = {selected:{getVoiceChannelId:()=>"100"}, voice:{getVoiceStatesForChannel:()=>({one:{userId:"9007199254740993"}})}, channels:{getChannel:()=>({name:"Lounge"})}};
  assert.equal(resolveUserCall(stores,"9007199254740993"),"Lounge");
  assert.equal(resolveUserCall(stores,"2"),null);
  stores.channels.getChannel=()=>null;
  assert.equal(resolveUserCall(stores,"9007199254740993"),null);
});

test("quiet surfaces exclude essential notices and settings controls", () => {
  const {document}=parseHTML('<html><body><div class="nitroUpsell_test" id="promo"></div><div role="alert"><div class="nitroUpsell_test" id="security"></div></div><div class="marketingBanner_test"><input></div><a href="/shop" id="shop">Shop</a><div class="avatarDecoration_test" id="decor"></div></body></html>');
  globalThis.document=document;
  assert.deepEqual(quietSurfaceNodes("promotions").map(node=>node.id),["promo"]);
  assert.deepEqual(quietSurfaceNodes("shop").map(node=>node.id),["shop"]);
  assert.deepEqual(quietSurfaceNodes("decorations").map(node=>node.id),["decor"]);
  assert.equal(quietSurfaceNodes("unknown"),null);
});

test("native status labels require exact user identity and link menus exclude users", () => {
  const {document}=parseHTML('<html><body><div data-user-id="9007199254740993"><span class="activity_test">In voice</span></div><div role="menu" id="user-context"><div role="menuitem" id="copy-link"></div></div><div role="menu" id="link-context"><div role="menuitem" id="link-copy-link"></div></div></body></html>');
  globalThis.document=document;
  assert.equal(voiceStatusLabels()[0].userId,"9007199254740993");
  assert.equal(nativeLinkMenuAnchor(document.querySelector('#user-context')),null);
  assert.equal(nativeLinkMenuAnchor(document.querySelector('#link-context')).id,"link-copy-link");
});