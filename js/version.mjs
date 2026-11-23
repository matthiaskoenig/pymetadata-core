// The version of the crate the module is built from, which the package must carry.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The root of the repository. */
export const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * The `version` of `[workspace.package]` in the root `Cargo.toml`.
 *
 * @returns {string}
 */
export function crateVersion() {
  const toml = readFileSync(join(root, "Cargo.toml"), "utf8");
  const section = toml.split(/^\[workspace\.package\]\s*$/m)[1]?.split(/^\[/m)[0] ?? "";
  const match = section.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("Cargo.toml has no version under [workspace.package]");
  return match[1];
}
