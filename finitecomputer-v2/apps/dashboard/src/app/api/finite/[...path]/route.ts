import { NextRequest, NextResponse } from "next/server";

const METHODS_WITH_BODY = new Set(["POST", "PUT", "PATCH"]);

export async function GET(request: NextRequest, context: RouteContext) {
  return proxyFiniteApi(request, context);
}

export async function POST(request: NextRequest, context: RouteContext) {
  return proxyFiniteApi(request, context);
}

export async function PUT(request: NextRequest, context: RouteContext) {
  return proxyFiniteApi(request, context);
}

type RouteContext = {
  params: Promise<{ path?: string[] }>;
};

async function proxyFiniteApi(request: NextRequest, context: RouteContext) {
  const baseUrl = process.env.FC_CORE_BASE_URL?.trim();
  if (!baseUrl) {
    return NextResponse.json({ error: "Core service is not configured." }, { status: 503 });
  }

  const { path = [] } = await context.params;
  const upstream = new URL(`/api/finite/${path.map(encodeURIComponent).join("/")}`, baseUrl);
  upstream.search = request.nextUrl.search;

  const headers = new Headers();
  const authorization = request.headers.get("authorization");
  if (authorization) {
    headers.set("authorization", authorization);
  }
  const contentType = request.headers.get("content-type");
  if (contentType) {
    headers.set("content-type", contentType);
  }

  const response = await fetch(upstream, {
    method: request.method,
    headers,
    body: METHODS_WITH_BODY.has(request.method) ? await request.arrayBuffer() : undefined,
    cache: "no-store",
  });
  const body = await response.arrayBuffer();
  const responseHeaders = new Headers();
  const responseContentType = response.headers.get("content-type");
  if (responseContentType) {
    responseHeaders.set("content-type", responseContentType);
  }
  return new NextResponse(body, {
    status: response.status,
    headers: responseHeaders,
  });
}
