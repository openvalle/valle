#!/usr/bin/env bun

import { mathCommand } from "./commands/math.ts";
import { browserCommand } from "./commands/browser.ts";
import {
  CliError,
  type CommandDefinition,
  formatCommandHelp,
  parseOptions,
} from "./lib/args.ts";

const commands: readonly CommandDefinition[] = [
  mathCommand,
  browserCommand,
];

function topLevelHelp(): string {
  return [
    "usage: bun web/tests/parity/cli.ts <command> [options]",
    "",
    "Valle Native/WASM/CanvasKit/browser parity tools.",
    "",
    "commands:",
    ...commands.map(({ name, summary }) => `  ${name.padEnd(16)} ${summary}`),
    "",
    "Run a command with --help for its options.",
  ].join("\n");
}

async function main(argv: readonly string[]): Promise<void> {
  const [name, ...rest] = argv;
  if (name === "--help" || name === "-h") {
    console.log(topLevelHelp());
    return;
  }
  if (!name) throw new CliError(`${topLevelHelp()}\n\nmissing command`, 2);
  const command = commands.find((candidate) => candidate.name === name);
  if (!command) throw new CliError(`unknown parity command ${JSON.stringify(name)}\n\n${topLevelHelp()}`, 2);
  try {
    await command.run(parseOptions(command, rest));
  } catch (error) {
    if (error instanceof CliError && error.exitCode === 0) {
      console.log(error.message || formatCommandHelp(command));
      return;
    }
    throw error;
  }
}

try {
  await main(Bun.argv.slice(2));
} catch (error) {
  const exitCode = error instanceof CliError ? error.exitCode : 1;
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = exitCode;
}
