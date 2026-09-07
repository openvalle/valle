export interface OptionDefinition {
  name: string;
  value: string;
  description: string;
  required?: boolean;
}

export class CliError extends Error {
  constructor(
    message: string,
    readonly exitCode = 1,
  ) {
    super(message);
    this.name = "CliError";
  }
}

export interface CommandDefinition {
  name: string;
  summary: string;
  options: readonly OptionDefinition[];
  run(options: ReadonlyMap<string, string>): Promise<void>;
}

export function parseOptions(
  command: CommandDefinition,
  argv: readonly string[],
): ReadonlyMap<string, string> {
  const known = new Map(command.options.map((option) => [option.name, option]));
  const parsed = new Map<string, string>();

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index]!;
    if (token === "--help" || token === "-h") {
      throw new CliError(formatCommandHelp(command), 0);
    }
    if (!token.startsWith("--")) {
      throw new CliError(`unexpected positional argument ${JSON.stringify(token)}`, 2);
    }

    const name = token.slice(2);
    if (!known.has(name)) throw new CliError(`unknown option --${name}`, 2);
    if (parsed.has(name)) throw new CliError(`option --${name} was provided more than once`, 2);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new CliError(`option --${name} requires a value`, 2);
    }
    parsed.set(name, value);
    index += 1;
  }

  const missing = command.options
    .filter(({ name, required }) => required && !parsed.has(name))
    .map(({ name }) => `--${name}`);
  if (missing.length) {
    throw new CliError(`missing required option${missing.length === 1 ? "" : "s"}: ${missing.join(", ")}`, 2);
  }
  return parsed;
}

export function requiredOption(options: ReadonlyMap<string, string>, name: string): string {
  const value = options.get(name);
  if (value === undefined) throw new CliError(`missing required option --${name}`, 2);
  return value;
}

export function formatCommandHelp(command: CommandDefinition): string {
  const usage = command.options
    .map(({ name, value, required }) => `${required ? "" : "["}--${name} <${value}>${required ? "" : "]"}`)
    .join(" ");
  const rows = command.options.map(
    ({ name, value, description }) => `  --${name} <${value}>\n      ${description}`,
  );
  return [`usage: bun web/tests/parity/cli.ts ${command.name} ${usage}`, "", command.summary, "", ...rows].join(
    "\n",
  );
}
