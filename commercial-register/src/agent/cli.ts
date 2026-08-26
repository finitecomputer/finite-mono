#!/usr/bin/env node

import { readFile } from 'node:fs/promises';

import { applyCommercialUpdate, showOrganization } from './register';
import { TwentyClient } from './twenty-client';
import { parseCommercialUpdate } from './validation';

async function main(argv: string[]): Promise<void> {
  const [command, ...args] = argv;
  if (command === 'apply') {
    const file = option(args, '--file');
    if (!file) throw new Error('apply requires --file <path> (or --file - for stdin)');
    const raw = file === '-' ? await readStdin() : await readFile(file, 'utf8');
    const update = parseCommercialUpdate(JSON.parse(raw));
    const result = await applyCommercialUpdate(clientFromEnvironment(), update);
    writeJson(result);
    return;
  }
  if (command === 'show') {
    const organization = option(args, '--organization');
    if (!organization) throw new Error('show requires --organization <name>');
    const result = await showOrganization(
      clientFromEnvironment(),
      organization,
    );
    writeJson(result);
    return;
  }
  throw new Error(
    'usage: finite-commercial-register apply --file <path>|- | show --organization <name>',
  );
}

function clientFromEnvironment(): TwentyClient {
  const baseUrl = process.env.FINITE_COMMERCIAL_TWENTY_URL;
  const apiKey = process.env.FINITE_COMMERCIAL_TWENTY_API_KEY;
  if (!baseUrl) throw new Error('FINITE_COMMERCIAL_TWENTY_URL is required');
  if (!apiKey) throw new Error('FINITE_COMMERCIAL_TWENTY_API_KEY is required');
  return new TwentyClient(baseUrl, apiKey);
}

function option(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  if (index === -1) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${name} requires a value`);
  }
  return value;
}

async function readStdin(): Promise<string> {
  let value = '';
  process.stdin.setEncoding('utf8');
  for await (const chunk of process.stdin) value += chunk;
  return value;
}

function writeJson(value: unknown): void {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

main(process.argv.slice(2)).catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`commercial register: ${message}\n`);
  process.exitCode = 1;
});
