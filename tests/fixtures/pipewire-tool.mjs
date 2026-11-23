#!/usr/bin/env node
import net from "node:net";
import path from "node:path";
import fs from "node:fs";

const tool = path.basename(process.argv[1]);
const mode = process.env.INTELLICORD_FIXTURE_MODE;
setTimeout(() => process.exit(25), 10000).unref();
const connection = net.createConnection(process.env.INTELLICORD_FIXTURE_SOCKET);
connection.on("connect", () => {
    connection.end(JSON.stringify({ tool, pid: process.pid }) + "\n");
    if (tool === "pw-dump") {
        if (mode === "invalid-metadata") {
            process.stdout.end("not JSON");
            return;
        }
        if (mode === "slow-discovery") {
            setInterval(() => {}, 1000);
            return;
        }
        const mediaClass = mode === "wrong-source" ? "Audio/Source" : "Audio/Sink";
        const continuityMarker = path.join(path.dirname(process.env.INTELLICORD_FIXTURE_SOCKET), "source-snapshot-seen");
        const continuityMode = mode === "source-replaced" || mode === "source-disappeared";
        const replaced = continuityMode && fs.existsSync(continuityMarker);
        if (continuityMode && !replaced) fs.writeFileSync(continuityMarker, "seen");
        if (mode === "source-disappeared" && replaced) {
            setTimeout(() => process.stdout.write("[]"), 100);
            return;
        }
        const snapshot = JSON.stringify([{
            type: "PipeWire:Interface:Node",
            info: { props: { "media.class": mediaClass, "object.serial": 42,
                "node.name": replaced ? "replacement-test-output" : "generated-test-output" } },
        }]);
        if (mode === "source-replaced" && replaced) setTimeout(() => process.stdout.write(snapshot), 100);
        else process.stdout.write(snapshot);
        return;
    }
    if (mode === "exit") {
        process.exitCode = 23;
        return;
    }
    if (mode === "eof") {
        fs.closeSync(1);
        setInterval(() => {}, 1000);
        return;
    }
    if (mode === "invalid") {
        const frame = Buffer.alloc(7680);
        frame.writeFloatLE(Number.NaN, frame.length - 4);
        process.stdout.write(frame);
    }
    if (mode === "stream" || mode === "analysis") {
        const frame = Buffer.alloc(7680);
        if (mode === "analysis") {
            for (let index = 0; index < 960; index += 1) {
                const sample = Math.sin(2 * Math.PI * 1000 * index / 48000) * 0.5;
                frame.writeFloatLE(sample, index * 8);
                frame.writeFloatLE(sample, index * 8 + 4);
            }
        }
        const emit = () => {
            if (!process.stdout.write(frame)) {
                process.stdout.once("drain", () => setTimeout(emit, 10));
            } else {
                setTimeout(emit, 10);
            }
        };
        emit();
    } else {
        setInterval(() => {}, 1000);
    }
});
connection.on("error", () => process.exit(24));