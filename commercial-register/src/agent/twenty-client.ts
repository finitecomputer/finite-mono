import type { TwentyRecord } from './types';

export class TwentyClient {
  readonly #baseUrl: string;
  readonly #apiKey: string;

  constructor(baseUrl: string, apiKey: string) {
    this.#baseUrl = baseUrl.replace(/\/+$/, '');
    this.#apiKey = apiKey;
  }

  async list(
    resource: string,
    filter?: { field: string; value: string },
  ): Promise<TwentyRecord[]> {
    const url = new URL(`${this.#baseUrl}/rest/${resource}`);
    url.searchParams.set('limit', '200');
    if (filter) {
      url.searchParams.set('filter', `${filter.field}[eq]:${filter.value}`);
    }
    const payload = await this.#request('GET', url);
    return extractRecords(payload, resource);
  }

  async create(
    resource: string,
    data: Record<string, unknown>,
  ): Promise<TwentyRecord> {
    const payload = await this.#request(
      'POST',
      new URL(`${this.#baseUrl}/rest/${resource}`),
      data,
    );
    return extractRecord(payload, resource);
  }

  async update(
    resource: string,
    id: string,
    data: Record<string, unknown>,
  ): Promise<TwentyRecord> {
    const payload = await this.#request(
      'PATCH',
      new URL(`${this.#baseUrl}/rest/${resource}/${encodeURIComponent(id)}`),
      data,
    );
    return extractRecord(payload, resource);
  }

  async #request(
    method: string,
    url: URL,
    data?: Record<string, unknown>,
  ): Promise<unknown> {
    const response = await fetch(url, {
      method,
      headers: {
        authorization: `Bearer ${this.#apiKey}`,
        accept: 'application/json',
        ...(data === undefined ? {} : { 'content-type': 'application/json' }),
      },
      body: data === undefined ? undefined : JSON.stringify(data),
    });
    const text = await response.text();
    let payload: unknown;
    try {
      payload = text === '' ? {} : JSON.parse(text);
    } catch {
      payload = { response: text.slice(0, 1_000) };
    }
    if (!response.ok) {
      throw new Error(
        `Twenty ${method} ${url.pathname} failed (${response.status}): ${JSON.stringify(payload)}`,
      );
    }
    return payload;
  }
}

function extractRecords(payload: unknown, resource: string): TwentyRecord[] {
  if (!isObject(payload)) throw new Error('Twenty returned a non-object response');
  const data = payload.data;
  const candidates = [
    data,
    isObject(data) ? data[resource] : undefined,
    payload[resource],
  ];
  for (const candidate of candidates) {
    if (Array.isArray(candidate)) return candidate.map(assertRecord);
    if (isObject(candidate) && Array.isArray(candidate.edges)) {
      return candidate.edges.map((edge) =>
        assertRecord(isObject(edge) ? edge.node : undefined),
      );
    }
  }
  if (isObject(data)) {
    const firstArray = Object.values(data).find(Array.isArray);
    if (Array.isArray(firstArray)) return firstArray.map(assertRecord);
  }
  return [];
}

function extractRecord(payload: unknown, resource: string): TwentyRecord {
  if (!isObject(payload)) throw new Error('Twenty returned a non-object response');
  const data = payload.data;
  if (isRecord(data)) return data;
  if (isObject(data)) {
    const direct = data[resource];
    if (isRecord(direct)) return direct;
    const record = Object.values(data).find(isRecord);
    if (record) return record;
  }
  if (isRecord(payload)) return payload;
  throw new Error('Twenty response did not contain a record');
}

function assertRecord(value: unknown): TwentyRecord {
  if (!isRecord(value)) throw new Error('Twenty response contained an invalid record');
  return value;
}

function isRecord(value: unknown): value is TwentyRecord {
  return isObject(value) && typeof value.id === 'string';
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
