import { constants } from "node:fs";
import { access, readdir, readFile, realpath } from "node:fs/promises";
import { dirname, join } from "node:path";
import { resolveExecutable } from "./common";

const OAUTH_SOURCE_PATH = join("dist", "src", "code_assist", "oauth2.js");

async function exists(path: string): Promise<boolean> {
  try {
    await access(path, constants.R_OK);
    return true;
  } catch {
    return false;
  }
}

function parseClientCredentials(
  source: string,
): { clientId: string; clientSecret: string } | null {
  const clientId = source.match(/OAUTH_CLIENT_ID\s*=\s*["']([^"']+)["']/)?.[1];
  const clientSecret = source.match(
    /OAUTH_CLIENT_SECRET\s*=\s*["']([^"']+)["']/,
  )?.[1];
  return clientId && clientSecret ? { clientId, clientSecret } : null;
}

async function readClientCredentials(
  path: string,
): Promise<{ clientId: string; clientSecret: string } | null> {
  try {
    return parseClientCredentials(await readFile(path, "utf8"));
  } catch {
    return null;
  }
}

async function packageRoot(binary: string): Promise<string | null> {
  let current = dirname(binary);
  for (let depth = 0; depth <= 8; depth += 1) {
    try {
      const manifest = JSON.parse(
        await readFile(join(current, "package.json"), "utf8"),
      ) as { name?: unknown };
      if (manifest.name === "@google/gemini-cli") return current;
    } catch {
      // Global installs often put the launcher several levels above the package.
    }
    for (const nested of [
      join(current, "lib", "node_modules", "@google", "gemini-cli"),
      join(current, "node_modules", "@google", "gemini-cli"),
    ]) {
      if (await exists(join(nested, "package.json"))) return nested;
    }
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return null;
}

export async function extractGeminiOAuthClient(
  configuredExecutable: string | null,
): Promise<{ clientId: string; clientSecret: string } | null> {
  const executable = await resolveExecutable("gemini", configuredExecutable);
  if (!executable) return null;
  const resolved = await realpath(executable).catch(() => executable);
  const binDirectory = dirname(resolved);
  const prefix = dirname(binDirectory);
  const candidates = [
    join(
      prefix,
      "libexec",
      "lib",
      "node_modules",
      "@google",
      "gemini-cli",
      "node_modules",
      "@google",
      "gemini-cli-core",
      OAUTH_SOURCE_PATH,
    ),
    join(
      prefix,
      "lib",
      "node_modules",
      "@google",
      "gemini-cli",
      "node_modules",
      "@google",
      "gemini-cli-core",
      OAUTH_SOURCE_PATH,
    ),
    join(prefix, "..", "gemini-cli-core", OAUTH_SOURCE_PATH),
    join(
      prefix,
      "node_modules",
      "@google",
      "gemini-cli-core",
      OAUTH_SOURCE_PATH,
    ),
  ];
  for (const candidate of candidates) {
    const credentials = await readClientCredentials(candidate);
    if (credentials) return credentials;
  }

  const root = await packageRoot(resolved);
  if (!root) return null;
  for (const candidate of [
    join(root, "node_modules", "@google", "gemini-cli-core", OAUTH_SOURCE_PATH),
    join(root, OAUTH_SOURCE_PATH),
  ]) {
    const credentials = await readClientCredentials(candidate);
    if (credentials) return credentials;
  }

  const bundle = join(root, "bundle");
  const entries = await readdir(bundle).catch(() => []);
  for (const entry of entries) {
    if (!entry.endsWith(".js")) continue;
    const credentials = await readClientCredentials(join(bundle, entry));
    if (credentials) return credentials;
  }
  return null;
}
