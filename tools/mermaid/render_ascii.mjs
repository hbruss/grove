#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { stdin, stdout, stderr, exit, argv } from "node:process";

const usage = "Usage: node tools/mermaid/render_ascii.mjs [<input-file>|-]";

async function main() {
  const input = await readInput(argv.slice(2));
  if (input === null) {
    stdout.write(`${usage}\n`);
    return;
  }
  const renderer = await loadRenderer();
  const rendered = await renderAscii(renderer, input);
  stdout.write(rendered.endsWith("\n") ? rendered : `${rendered}\n`);
}

async function readInput(args) {
  const [firstArg, ...rest] = args;
  if (rest.length > 0) {
    throw new Error(usage);
  }

  if (firstArg === "--help" || firstArg === "-h") {
    return null;
  }

  if (!firstArg || firstArg === "-") {
    if (stdin.isTTY) {
      throw new Error(usage);
    }
    return readStdin();
  }

  return readFile(firstArg, "utf8");
}

async function readStdin() {
  return await new Promise((resolve, reject) => {
    let data = "";
    stdin.setEncoding("utf8");
    stdin.on("data", (chunk) => {
      data += chunk;
    });
    stdin.on("end", () => resolve(data));
    stdin.on("error", reject);
  });
}

async function loadRenderer() {
  try {
    const module = await import("beautiful-mermaid");
    const renderMermaidAscii =
      module.renderMermaidAscii ?? module.default?.renderMermaidAscii;
    if (typeof renderMermaidAscii !== "function") {
      throw new Error("beautiful-mermaid does not export renderMermaidAscii");
    }
    return renderMermaidAscii;
  } catch (error) {
    throw new Error(
      "beautiful-mermaid is not installed in tools/mermaid/node_modules yet"
    );
  }
}

async function renderAscii(renderMermaidAscii, source) {
  const output = await renderMermaidAscii(source);
  if (typeof output === "string") {
    return output;
  }
  if (output && typeof output === "object") {
    if (typeof output.ascii === "string") {
      return output.ascii;
    }
    if (typeof output.output === "string") {
      return output.output;
    }
    if (typeof output.text === "string") {
      return output.text;
    }
  }
  return String(output ?? "");
}

main().catch((error) => {
  stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  stderr.write(`${usage}\n`);
  exit(1);
});
