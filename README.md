# Intellicord

A Rust-backed, opt-in enhancement project for the Discord desktop client. Callmon is one of its first major features: a small JavaScript adapter observes the client's existing voice stores and renders a compact menu while Rust computes and retains join/leave history. No BetterDiscord or Vencord installation is required, but Discord must explicitly expose its Chromium DevTools endpoint.

**Status:** working core, injector, and interactive preview. Live attachment, active-call detection, and observed membership activity were verified on Discord stable desktop 1.0.169, web build 673869. Other releases and call types remain unverified. Discord has no official vanilla-client plugin API. Its private stores, module cache, and debugging flags may change or be unavailable. Client modifications are unsupported and may violate Discord's terms or create account risk; review those terms before using this with an account.

## Build

Requires a recent Rust toolchain supporting edition 2024, Node.js 20+, and npm.

```sh
npm ci
npm run build
cargo build
```

Build the JavaScript first: Rust embeds `dist/client.js` in the executable at compile time. Rebuild both after changing the adapter or menu. The compiled executable needs neither Node.js nor a separate web server at runtime.

Open [preview.html](preview.html) after building the JavaScript for an interactive preview with explicitly labeled demo data. It uses the same menu as the injected plugin. No Discord login or server is needed for the preview.

The Rust package and executable are named `intellicord`, and the private JavaScript package is `intellicord-client`. Callmon remains the voice-history feature name, including its text-log filenames. The internal `__callmon_send` binding and `__callmon` lifecycle alias are retained for transport compatibility; `__intellicord` owns the combined feature lifecycle.

## Attach To Discord

On Linux, run `./scripts/install-user.sh` to install `~/.local/bin/intellicord` and a user-local Discord desktop launcher that runs `intellicord launch`. Subsequent launches attach automatically using loopback-only DevTools. An already-running non-debuggable Discord instance must be quit and relaunched by you; the installer never changes running processes. Updating the installed binary does not restart the active backend, so backend protocol changes take effect on its next launch. `./scripts/uninstall-user.sh` removes the managed launcher and binary and restores a backed-up custom launcher, without stopping Discord.

For manual attachment:

1. Fully quit Discord, including its tray process. Starting another instance usually forwards to the existing process and ignores debugging flags.
2. Start Discord yourself with a loopback-only debugging endpoint:

   ```sh
   discord --remote-debugging-address=127.0.0.1 --remote-debugging-port=9222
   ```

3. Log in normally and wait for the client to load. Confirm the endpoint is not listening on a public/network interface:

   ```sh
   ss -ltnp '( sport = :9222 )'
   ```

4. In a separate terminal, attach:

   ```sh
   cargo run -- inject --port 9222
   ```

5. Open a voice call view and use the Callmon history icon at the bottom-left of that call. Keep the Rust process running. Ctrl+C removes injected features, listeners, and binding. Restart Discord without debugging flags afterward to close the endpoint. The executable is `target/debug/intellicord`; the workspace directory need not be renamed.

The endpoint grants full access to your logged-in renderer. **Never expose or forward it to another machine.** The injector connects only to a loopback websocket on the specified port and accepts only Discord app/channel pages. Those checks do not make an exposed DevTools endpoint safe. No tokens, passwords, or account credentials are required by Intellicord.

If several Discord pages are available, the injector prints their target IDs; select one with `--target ID`. If your build disables remote debugging or does not expose the required webpack cache/stores, this prototype cannot attach. It deliberately does not patch installed files, bypass those restrictions, or enable Node integration.

After a Discord page reload, run the injector again. If the backend was forcibly terminated, reload Discord first to discard its orphaned menu. A second injector refuses to replace an existing instance.

## Behavior And Privacy

- Records observed joins and leaves in the user's currently connected voice channel, including users moving into or out of that channel. It does not monitor other channels.
- Existing participants establish a baseline when attaching, joining, or switching calls. They are not falsely logged as new joins. Disconnecting or switching your own call does not fabricate departures for everyone in the previous call.
- Retains the latest 500 events across observed calls for this injector session. Previous-channel entries retain their channel names. History is memory-only, with no disk persistence, telemetry, uploads, or historical backfill.
- Displays global display names and usernames as observed at the time of an event; server nicknames are not yet supported. IDs remain strings end to end, avoiding JavaScript numeric precision loss.
- Shows observation timestamps, newest first, with All/Joined/Left filters and username/ID/channel search. Copy buttons use the clipboard, with a selectable-ID fallback if clipboard access is denied. Clearing history requires confirmation and keeps the current membership baseline.
- The user-filter icon opens a searchable checklist of recorded users and current callers. Selection uses exact user IDs, combines with the search and event-type filters, and does not alter history. All users restores the full selection; selecting nobody produces no matching events. Current callers appear even before any activity has been recorded, without fabricating join events.
- The download icon exports only matching events to a UTF-8 text log, oldest first, with UTC ISO timestamps, join/leave actions, usernames, display names, exact user IDs, and channel details. Fields are escaped to prevent names from forging log entries. Export is disabled when nothing matches. This explicit download is the exception to memory-only storage; saving is handled by the client/browser, and downloaded files remain until you delete them.
- Right-click an activity entry or user-checklist row to open Discord's actual user context menu. Shift+F10 and the Context Menu key work on focused rows too. Discord supplies the profile, message, mute, volume, moderation, and other actions according to the user, channel, and your permissions. Historical entries retain their original channel context; unavailable users/channels and unsupported native menu APIs show a visible error while history remains usable. Callmon does not undo actions taken from this menu.
- User pictures use Discord's native avatar URL with a validated avatar-hash fallback. Only HTTPS Discord CDN/media profile-image paths are accepted, image requests omit the referrer, and initials remain available when an image cannot load. Displaying pictures makes normal CDN image requests; Callmon does not persist pictures or identities to disk.
- Uses Discord font/color variables where available, compact spacing, muted metadata, familiar icons, and small-radius surfaces. Callmon's launcher follows the bottom-left of the visible call view, not the application corner; its popup is clamped to the viewport. It hides when the call view is absent or user settings are open. Scroll, viewport changes, call-view replacement, and element resizing refresh its position.
- Listens to voice and selected-channel store changes, with a one-second reconciliation poll. Very short transitions that are not exposed by those stores can be missed. Missing users or unsupported snapshots pause capture instead of producing false departures. Recovery records the next observable difference, not unobserved intermediate activity.
- Captures no audio, messages, friend lists, or authentication tokens. Usernames and IDs are still personal data; use the history responsibly and avoid sharing it unnecessarily.

## Sponsored Content

Intellicord adds **Disable Quests and Sponsored Content** at the head of **Settings > Data & Privacy > Sponsored Content**. It is off by default. Enabling it hides recognized Quest and sponsored UI surfaces, including supported cards, prompts, and navigation links, and continues filtering elements added after navigation. Turning it off or detaching restores the elements. The switch remains accessible, and Discord's native privacy controls are excluded from filtering.

This is **client-side UI suppression only**, not an account-wide or network-level disable. It does not stop Discord fetching content, measuring activity, progressing accepted Quests, or sending notifications outside the injected renderer. Discord's two native personalization opt-outs explicitly say Quests may still appear; Intellicord does not change those settings or claim a complete service-level opt-out. Keep using native privacy settings for personalization choices. Other surfaces or future builds may need new adapter selectors.

Only this preference is persisted as a boolean in `$XDG_STATE_HOME/intellicord/preferences.json` (default `~/.local/state/intellicord/preferences.json`) by the Rust backend; no call history or identity data is written there. On Linux, the directory is owner-only and the file uses mode 0600. This works even when Discord denies renderer localStorage access. The switch waits for backend acknowledgement before changing suppression. Storage failures display an error and leave the setting unchanged. The preference stays saved when detaching, but suppression applies only while Intellicord is attached. It is local to this OS user, not synchronized to your Discord account.

## Verification

```sh
npm test
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Rust tests cover baseline/diff semantics, exact IDs, channel switches, disconnects, bounded history, invalid snapshots, clear behavior, target restrictions, script errors, and early binding events over a real loopback websocket. JavaScript tests also cover store discovery, Rspack async exports, multiple runtimes, translation proxies, named-store selection over helpers, duplicate snapshots, cleanup, invalid/unresolved participants, user selection, baseline callers, chronological log formatting, escaped fields, native mute dispatch/subscriptions, and self/absent-user guards. Browser checks performed during development cover filters, selected-user export payloads and filenames, empty-export disabling, simulated mute/unmute synchronization, baseline mute controls, exact-ID copying, clipboard denial, confirmed clear, Escape, long names, desktop/mobile layout, and short-window layout.

Live right-click integration rendered Discord's actual user menu, including mute and volume controls, on the tested build. The menu was closed without selecting any action; actual audio muting and saving an exported file through Discord's download handling remain unverified. Browser export checks captured the generated payload and filename rather than completing a file save. The frontend update preserved all 19 existing history events without restarting Discord or the backend. Native menu discovery and row/keyboard delegation also have mocked regression coverage. The standalone preview simulates user actions; profile and message actions are disabled there because it has no authenticated Discord session.

Additional tests cover call-relative placement and viewport constraints, opt-in preference persistence, malformed/denied storage, settings placement, privacy-control exclusions, dynamically added Quest elements, navigation, and detach restoration. Browser checks cover sponsored demo suppression, reload persistence, restoration, unchanged native controls, and responsive settings layout. Live inspection verified the Sponsored Content anchor and injected switch placement on the tested Discord build; filtering every real Quest/sponsored surface remains unverified. The live upgrade retained all 27 history events without restarting the backend.

## Code

- [src/lib.rs](src/lib.rs): typed snapshots and bounded membership history.
- [src/main.rs](src/main.rs): target discovery, DevTools transport, injection, heartbeat, and cleanup.
- [src/web/bridge.js](src/web/bridge.js): guarded store discovery and voice observation.
- [src/web/client.js](src/web/client.js): injection entry point and lifecycle.
- [src/web/activity.js](src/web/activity.js): user selection, matching events, and text-log formatting.
- [src/web/placement.js](src/web/placement.js): call-relative launcher and popup geometry.
- [src/web/sponsored.js](src/web/sponsored.js) and [src/web/sponsored.css](src/web/sponsored.css): opt-in content filtering and native-styled settings control.
- [src/web/preferences.js](src/web/preferences.js): local boolean preference storage.
- [src/web/menu.js](src/web/menu.js) and [src/web/menu.css](src/web/menu.css): shared menu and styling.
- [src/web/preview.js](src/web/preview.js): standalone demo fixture, not live call capture.

The next integration milestone is validating additional Discord releases and call types. Durable storage and native toolbar integration are intentionally deferred.