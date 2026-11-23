// Run by `npm pack` and `npm publish`: refuse a package which is not what the crate is.
import { copyFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

import { crateVersion, root } from "./version.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const version = JSON.parse(readFileSync(join(here, "package.json"), "utf8")).version;
if (version !== crateVersion()) {
  console.error(`package.json is ${version} but the crate is ${crateVersion()}; make them equal`);
  process.exit(1);
}
// the module must be the current build of the sources, not a stale one
execFileSync("node", [join(here, "build.mjs"), "--check"], { stdio: "inherit" });
// the license of the repository travels with the package
copyFileSync(join(root, "LICENSE"), join(here, "LICENSE"));
