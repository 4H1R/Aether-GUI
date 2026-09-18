import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmod, cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(await readFile(join(root, "aether-core.json"), "utf8"));

export function selectAsset(platform, arch, override) {
  const asset = override
    ? Object.values(manifest.assets).find((entry) => entry.name === override)
    : manifest.assets[`${platform}-${arch}`];
  if (!asset) throw new Error(`Unsupported Aether target: ${override || `${platform}-${arch}`}`);
  return asset;
}

export function verifyArchive(bytes, expected) {
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) throw new Error(`Aether checksum mismatch: ${actual} != ${expected}`);
}

async function makeExecutable(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await makeExecutable(path);
    else await chmod(path, 0o755);
  }
}

export async function fetchAether() {
  const asset = selectAsset(process.platform, process.arch, process.env.AETHER_ASSET);
  const binaries = join(root, "src-tauri", "binaries");
  await mkdir(binaries, { recursive: true });
  const temporary = await mkdtemp(join(binaries, ".fetch-"));
  try {
    const url = `https://github.com/${manifest.repository}/releases/download/${manifest.version}/${asset.name}`;
    console.log(`Downloading Aether ${manifest.version}: ${asset.name}`);
    const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw new Error(`Download failed: HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    verifyArchive(bytes, asset.sha256);
    const archive = join(temporary, asset.name);
    await writeFile(archive, bytes);
    const extracted = join(temporary, "extracted");
    await mkdir(extracted);
    // Windows ships bsdtar, which also handles ZIP archives.
    execFileSync("tar", ["-xf", archive, "-C", extracted]);
    const name = asset.name.includes("windows") ? "aether.exe" : "aether";
    if (!(await stat(join(extracted, name))).isFile()) throw new Error("Missing Aether executable");
    if (!(await stat(join(extracted, "pt"))).isDirectory()) throw new Error("Missing Tor transports");
    if (!asset.name.includes("windows")) await makeExecutable(extracted);
    // Replace only the generated resource directory after full verification.
    const destination = join(binaries, "core");
    await rm(destination, { recursive: true, force: true });
    await mkdir(destination);
    await cp(join(extracted, name), join(destination, name));
    await cp(join(extracted, "pt"), join(destination, "pt"), { recursive: true });
    await writeFile(join(destination, "version.txt"), `${manifest.version}\n`);
    console.log(`Verified core and transports ready in ${destination}`);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await fetchAether();
}
