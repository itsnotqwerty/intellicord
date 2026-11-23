import { appendFileSync, copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

const tool = basename(process.argv[1]);
const args = process.argv.slice(2);
appendFileSync(process.env.INTELLICORD_INSTALL_TEST_LOG, `${JSON.stringify({ tool, args })}\n`);

if (tool === "rustup") {
  if (args.includes("component")) process.stdout.write("rust-src\n");
  if (args.includes("build")) {
    const targetDir = args[args.indexOf("--target-dir") + 1];
    const object = join(targetDir, "bpfel-unknown-none", "release", "intellicord-process");
    mkdirSync(join(targetDir, "bpfel-unknown-none", "release"), { recursive: true });
    writeFileSync(object, "generated eBPF object");
  }
} else if (tool === "cargo") {
  if (args[0] === "run") {
    process.stdout.write('{"source":"generated_install_fixture","sensor_running":false}\n');
  } else if (args[0] === "build") {
    const release = join(process.env.CARGO_TARGET_DIR, "release");
    mkdirSync(release, { recursive: true });
    writeFileSync(join(release, "intellicord"), "generated injector", { mode: 0o755 });
    if (args.includes("observe")) {
      writeFileSync(join(release, "intellicord-observe"), "generated observer", { mode: 0o755 });
    }
  }
} else if (tool === "desktop-file-install") {
  const directory = args.find(value => value.startsWith("--dir=")).slice(6);
  mkdirSync(directory, { recursive: true });
  copyFileSync(args.at(-1), join(directory, "discord.desktop"));
}