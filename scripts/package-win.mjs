#!/usr/bin/env node
/**
 * Recolhe o instalador Windows recém-buildado para `dist-windows/`.
 *
 * Roda logo depois do `tauri build` (ver o script `build:win`), porque a cópia
 * era feita à mão e a pasta foi acumulando versões: uma 0.1.0 ficou lá ao lado
 * da 0.2.0 sem que nada indicasse qual era a atual.
 *
 * `dist-windows/` guarda **apenas a última versão buildada**. O conteúdo
 * anterior é apagado a cada execução — é um diretório de saída, não um
 * arquivo histórico. Releases antigas se reconstroem a partir do git; a pasta
 * está no `.gitignore` e nunca foi publicada.
 *
 * A versão vem do `VERSION` (ver `scripts/version.mjs`), a mesma que o
 * `package.json` entrega ao Tauri — então um descompasso entre o nome do
 * arquivo buildado e a versão esperada é erro, não algo a contornar.
 */
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "dist-windows");
const BUNDLE_DIR = join(ROOT, "src-tauri", "target", "x86_64-pc-windows-msvc", "release", "bundle");

function fail(message) {
  console.error(`package-win: ${message}`);
  process.exit(1);
}

const version = readFileSync(join(ROOT, "VERSION"), "utf8").trim();

/** Instaladores produzidos por este build, em qualquer bundler que o Tauri usou. */
function builtInstallers() {
  const found = [];
  for (const bundler of ["nsis", "msi"]) {
    const dir = join(BUNDLE_DIR, bundler);
    let entries;
    try {
      entries = readdirSync(dir);
    } catch {
      continue; // bundler não usado neste build
    }
    for (const name of entries) {
      if (name.endsWith(".exe") || name.endsWith(".msi")) {
        found.push({ dir, name });
      }
    }
  }
  return found;
}

const all = builtInstallers();
if (all.length === 0) {
  fail(`nenhum instalador em ${BUNDLE_DIR}. Rodou o \`tauri build\` antes?`);
}

// O diretório de bundle do Tauri também acumula: filtrar pela versão atual é o
// que impede publicar o instalador de um build anterior por engano.
const current = all.filter(({ name }) => name.includes(`_${version}_`));
if (current.length === 0) {
  fail(
    `nenhum instalador da versão ${version}. Encontrados: ${all.map((f) => f.name).join(", ")}`,
  );
}

rmSync(OUT_DIR, { recursive: true, force: true });
mkdirSync(OUT_DIR, { recursive: true });

for (const { dir, name } of current) {
  const target = join(OUT_DIR, name);
  copyFileSync(join(dir, name), target);

  const digest = createHash("sha256").update(readFileSync(target)).digest("hex");
  // Mesmo formato do `shasum -a 256`, para `shasum -c` funcionar direto.
  writeFileSync(`${target}.sha256`, `${digest}  ${name}\n`);

  console.log(`package-win: ${name}`);
  console.log(`             sha256 ${digest}`);
}

console.log(`package-win: dist-windows/ contém apenas a ${version}.`);
