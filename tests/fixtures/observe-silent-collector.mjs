#!/usr/bin/env node
import net from "node:net";

const argumentsList = process.argv.slice(3);
const value = name => argumentsList[argumentsList.indexOf(name) + 1];
const progress = net.createConnection(value("--socket"));
progress.on("error", () => process.exit(1));
progress.once("connect", () => {
  const control = net.createConnection(value("--object"));
  control.on("error", () => process.exit(1));
  control.once("connect", () => control.end(`${process.pid}\n`));
});