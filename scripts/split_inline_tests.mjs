#!/usr/bin/env node
// One-shot codemod: move a trailing inline `#[cfg(test)] mod tests { ... }`
// block to a sibling `<stem>_tests.rs` and replace it with a `#[path]` stub.
//
// Safe ONLY when the test module is the LAST item in the file (its closing
// brace is the final non-empty line) — no brace matching is performed; the
// split is purely line-range based, which avoids miscounting braces that
// appear inside string literals.
//
// Usage: node scripts/split_inline_tests.mjs <file.rs> <testStartLine>
//   <testStartLine> = 1-based line number of the `#[cfg(test)]` attribute.
import { readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const [file, startArg] = process.argv.slice(2);
if (!file || !startArg) {
  console.error("usage: split_inline_tests.mjs <file.rs> <testStartLine>");
  process.exit(2);
}
const start = Number(startArg); // 1-based line of `#[cfg(test)]`
const src = readFileSync(file, "utf8");
const lines = src.split("\n");
// Drop a single trailing empty element from a final newline so indexes line up.
const hadTrailingNewline = lines.length > 0 && lines[lines.length - 1] === "";
if (hadTrailingNewline) lines.pop();

const i = start - 1; // 0-based index of `#[cfg(test)]`
const attr = (lines[i] ?? "").trim();
const modLine = (lines[i + 1] ?? "").trim();
const last = (lines[lines.length - 1] ?? "").trim();
if (attr !== "#[cfg(test)]") {
  console.error(`FAIL: line ${start} is not '#[cfg(test)]' (got: ${JSON.stringify(attr)})`);
  process.exit(1);
}
const modMatch = modLine.match(/^mod\s+(\w+)\s*\{$/);
if (!modMatch) {
  console.error(`FAIL: line ${start + 1} is not 'mod <name> {' (got: ${JSON.stringify(modLine)})`);
  process.exit(1);
}
const modName = modMatch[1];
if (last !== "}") {
  console.error(`FAIL: last line is not a bare '}' (got: ${JSON.stringify(last)}) — module is not trailing`);
  process.exit(1);
}

const head = lines.slice(0, i); // everything before `#[cfg(test)]`
const inner = lines.slice(i + 2, lines.length - 1); // between `mod tests {` and final `}`

const stem = basename(file).replace(/\.rs$/, "");
const testFileName = `${stem}_tests.rs`;
const testFile = join(dirname(file), testFileName);

const hasSuper = inner.slice(0, 5).some((l) => l.trim() === "use super::*;");
// Keep the body verbatim (still indented one level); `cargo fmt` normalizes it.
const body = hasSuper ? inner : ["use super::*;", "", ...inner];

const stub = ["#[cfg(test)]", `#[path = "${testFileName}"]`, `mod ${modName};`];
const newHead = [...head, ...stub, ""].join("\n");
writeFileSync(file, newHead);
writeFileSync(testFile, `${body.join("\n")}\n`);

console.log(`OK ${file}: head=${head.length} lines, moved ${inner.length} test lines -> ${testFile}`);
