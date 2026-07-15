import { readFile, writeFile } from "node:fs/promises";

const bindings = process.argv.slice(2);

if (bindings.length === 0) {
  throw new Error("expected at least one generated binding path");
}

for (const binding of bindings) {
  const source = await readFile(binding, "utf8");
  const normalized = source.replace(/[\t ]+$/gm, "");
  if (normalized !== source) {
    await writeFile(binding, normalized, "utf8");
  }
}
