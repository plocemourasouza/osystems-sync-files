#!/usr/bin/env node
/**
 * Fonte única da versão do app.
 *
 * O número vivia em quatro manifestos e subia à mão nos quatro; esquecer um
 * fazia o instalador sair com nome divergente do `package.json`. Agora ele vive
 * em `VERSION`, na raiz, e os demais derivam:
 *
 *   VERSION  ──(este script)──▶  package.json
 *            └─(este script)──▶  src-tauri/Cargo.toml  [workspace.package]
 *                                        │
 *                                        └─(herança do Cargo)──▶ crates/core
 *   package.json ──(tauri.conf.json: "version": "../package.json")──▶ instalador
 *
 * Ou seja: dois lugares escritos por script, dois que herdam sozinhos. Nenhum
 * é editado à mão.
 *
 * Por que `VERSION` e não `.env`: `.env` e `.env.*` estão no `.gitignore` como
 * segredos (auditoria T-6.3 MINOR #10). Um arquivo ignorado não serve de fonte
 * de versão — o clone e o CI não a enxergariam.
 *
 * Uso:
 *   node scripts/version.mjs            # sincroniza os derivados a partir de VERSION
 *   node scripts/version.mjs --check    # falha se algum derivado divergir (CI)
 *   node scripts/version.mjs 0.3.0      # grava VERSION e sincroniza
 */
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const VERSION_FILE = join(ROOT, "VERSION");
const PACKAGE_JSON = join(ROOT, "package.json");
const WORKSPACE_TOML = join(ROOT, "src-tauri", "Cargo.toml");

/** Aceita apenas `x.y.z` — o NSIS e o MSI rejeitam sufixos como `-beta`. */
const SEMVER = /^\d+\.\d+\.\d+$/;

/** Só a `version` do bloco `[workspace.package]`, nunca a de um `[package]`. */
const WORKSPACE_VERSION = /(\[workspace\.package\][^[]*?\nversion = ")([^"]+)(")/;

function fail(message) {
  console.error(`version: ${message}`);
  process.exit(1);
}

function readVersionFile() {
  const raw = readFileSync(VERSION_FILE, "utf8").trim();
  if (!SEMVER.test(raw)) {
    fail(`VERSION contém "${raw}"; esperado x.y.z (sem prefixo "v", sem sufixo)`);
  }
  return raw;
}

/** @returns {{label: string, current: string, write: (v: string) => void}[]} */
function derived() {
  return [
    {
      label: "package.json",
      current: JSON.parse(readFileSync(PACKAGE_JSON, "utf8")).version,
      write(next) {
        // Reescrita textual, não `JSON.stringify` do objeto inteiro: preserva a
        // ordem das chaves e o formato que o npm já escreveu.
        const raw = readFileSync(PACKAGE_JSON, "utf8");
        writeFileSync(PACKAGE_JSON, raw.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${next}"`));
      },
    },
    {
      label: "src-tauri/Cargo.toml [workspace.package]",
      current: readFileSync(WORKSPACE_TOML, "utf8").match(WORKSPACE_VERSION)?.[2],
      write(next) {
        const raw = readFileSync(WORKSPACE_TOML, "utf8");
        if (!WORKSPACE_VERSION.test(raw)) {
          fail("não achei `version` sob [workspace.package] em src-tauri/Cargo.toml");
        }
        writeFileSync(WORKSPACE_TOML, raw.replace(WORKSPACE_VERSION, `$1${next}$3`));
      },
    },
  ];
}

const args = process.argv.slice(2);
const check = args.includes("--check");
const explicit = args.find((a) => !a.startsWith("-"));

if (explicit) {
  if (!SEMVER.test(explicit)) fail(`"${explicit}" não é x.y.z`);
  if (check) fail("--check e uma versão explícita são mutuamente exclusivos");
  writeFileSync(VERSION_FILE, `${explicit}\n`);
}

const version = readVersionFile();
const targets = derived();

if (check) {
  const drifted = targets.filter((t) => t.current !== version);
  if (drifted.length > 0) {
    console.error(`version: VERSION diz ${version}, mas divergem:`);
    for (const t of drifted) console.error(`  ${t.label}: ${t.current}`);
    console.error("\nRode `npm run version:sync` e faça commit do resultado.");
    process.exit(1);
  }
  console.log(`version: ${version} — package.json e Cargo em dia.`);
  process.exit(0);
}

let changed = 0;
for (const t of targets) {
  if (t.current === version) continue;
  t.write(version);
  console.log(`version: ${t.label} ${t.current} -> ${version}`);
  changed += 1;
}
console.log(
  changed === 0
    ? `version: ${version} — nada a fazer.`
    : `version: ${version} — ${changed} arquivo(s) atualizado(s). crates/core e o instalador herdam.`,
);
