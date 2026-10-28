#!/usr/bin/env node
// Writes THIRD-PARTY-NOTICES.txt: the license text of every Rust crate linked into the
// macOS binary and every npm package bundled into the frontend. Run it after changing
// dependencies (`npm run notices`); the release script refuses to ship a stale file.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const LICENSE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-._].*)?$/i;

function licenseTexts(dir) {
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f))
    .sort()
    .map((f) => `--- ${f} ---\n${readFileSync(join(dir, f), "utf8").trim()}`);
}

// Rust: crates reachable from the app through normal (non-dev, non-build) dependencies.
function rustPackages() {
  const meta = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--filter-platform", "aarch64-apple-darwin"],
      { cwd: join(root, "src-tauri"), maxBuffer: 1 << 28 },
    ),
  );
  const packages = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const stack = [meta.resolve.root];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id).deps) {
      if (dep.dep_kinds.some((k) => k.kind === null)) stack.push(dep.pkg);
    }
  }
  seen.delete(meta.resolve.root);
  return [...seen].map((id) => {
    const p = packages.get(id);
    return {
      name: p.name,
      version: p.version,
      license: p.license ?? "(sin declarar)",
      authors: p.authors.map((a) => a.replace(/\s*<[^>]*>/, "")),
      source: p.repository ?? `https://crates.io/crates/${p.name}`,
      texts: licenseTexts(dirname(p.manifest_path)),
    };
  });
}

// npm: runtime dependencies that Vite bundles into the frontend.
function npmPackages() {
  const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  return Object.keys(pkg.dependencies ?? {}).map((name) => {
    const dir = join(root, "node_modules", name);
    const meta = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const repo = typeof meta.repository === "string" ? meta.repository : meta.repository?.url;
    return {
      name,
      version: meta.version,
      license: meta.license ?? "(sin declarar)",
      source: repo ?? `https://www.npmjs.com/package/${name}`,
      texts: existsSync(dir) ? licenseTexts(dir) : [],
    };
  });
}

// Standard texts for crates published without a license file. MIT and BSD carry the
// authors' copyright line; MPL-2.0 is borrowed from a crate that ships it verbatim.
const MIT = (holder) => `MIT License

Copyright (c) ${holder}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;
const BSD3 = (holder) => `BSD 3-Clause License

Copyright (c) ${holder}

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`;

const all = [...rustPackages(), ...npmPackages()].sort(
  (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
);
// Dual-licensed crates are taken under MIT, so only their MIT text is needed.
for (const p of all) {
  if (!/\bMIT\b/.test(p.license) || !/\bOR\b|\//.test(p.license)) continue;
  const mit = p.texts.filter((t) => /^--- [^\n]*MIT/i.test(t));
  if (mit.length) p.texts = mit;
}
const mpl = all.find((p) => p.license === "MPL-2.0" && p.texts.length)?.texts[0];
for (const p of all.filter((p) => p.texts.length === 0)) {
  const holder = `${p.authors?.join(", ") || `the ${p.name} authors`}`;
  if (/\bMIT\b/.test(p.license)) p.texts = [`--- MIT (texto estándar) ---\n${MIT(holder)}`];
  else if (/BSD-3-Clause/.test(p.license)) p.texts = [`--- BSD-3-Clause (texto estándar) ---\n${BSD3(holder)}`];
  else if (p.license === "MPL-2.0" && mpl) p.texts = [mpl];
}
const missing = all.filter((p) => p.texts.length === 0);

// Packages sharing an identical license text are listed together under one copy of it.
const groups = new Map();
for (const p of all) {
  const text = p.texts.join("\n\n") || `Texto completo de la licencia: https://spdx.org/licenses/ (${p.license})`;
  const key = text.replace(/^--- .* ---$/gm, "").replace(/\s+/g, " ").trim();
  if (!groups.has(key)) groups.set(key, { text, pkgs: [] });
  groups.get(key).pkgs.push(p);
}

const out = [
  "Snapclone incluye software de terceros. Sus licencias y avisos se reproducen a continuación.",
  "Snapclone includes third-party software. Their licenses and notices are reproduced below.",
  "",
  `${all.length} paquetes. Generado con scripts/third-party-notices.mjs.`,
  "",
  ...[...groups.values()].flatMap(({ text, pkgs }) => [
    "=".repeat(78),
    ...pkgs.map((p) => `${p.name} ${p.version} (${p.license}) — ${p.source}`),
    "",
    text,
    "",
  ]),
].join("\n");

writeFileSync(join(root, "THIRD-PARTY-NOTICES.txt"), out);
console.log(`THIRD-PARTY-NOTICES.txt: ${all.length} paquetes, ${groups.size} textos, ${missing.length} sin licencia`);
for (const p of missing) console.log(`  sin licencia: ${p.name} ${p.version} (${p.license})`);
