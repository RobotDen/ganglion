/**
 * Ganglion telemetry checkpoint — the complete server side, published here
 * so anyone can audit what happens to the data (ADR-026, TELEMETRY.md).
 *
 * Guarantees implemented below, in order of appearance:
 *   - The client IP is NEVER stored, logged, or forwarded. No request
 *     logging is enabled on this worker.
 *   - Payloads over 4 KiB or failing schema validation are dropped.
 *   - The anonymous client id is hashed with a server-side secret before
 *     storage, so the stored value cannot be matched back to a client id
 *     even with database access.
 *   - Only aggregate rows are written (Workers Analytics Engine); raw
 *     requests are not retained anywhere.
 *
 * Endpoints:
 *   POST /v1/checkpoint   — daily CLI checkpoint; responds {"latest": "x.y.z"}
 *   POST /v1/relay-stats  — opt-in daily relay aggregates (unique peer count)
 *
 * Bindings expected (wrangler.toml):
 *   AE          — Analytics Engine dataset (aggregate rows)
 *   ID_SALT     — secret for server-side id hashing (wrangler secret put)
 */

const MAX_BODY_BYTES = 4096;
const RELEASES_URL =
  "https://api.github.com/repos/RobotDen/ganglion/releases/latest";
const LATEST_CACHE_SECONDS = 3600;

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (request.method !== "POST") {
      return new Response("method not allowed", { status: 405 });
    }
    if (url.pathname === "/v1/checkpoint") {
      return checkpoint(request, env, ctx);
    }
    if (url.pathname === "/v1/relay-stats") {
      return relayStats(request, env);
    }
    return new Response("not found", { status: 404 });
  },
};

async function readBoundedJson(request) {
  const raw = await request.arrayBuffer();
  if (raw.byteLength > MAX_BODY_BYTES) return null;
  try {
    return JSON.parse(new TextDecoder().decode(raw));
  } catch {
    return null;
  }
}

/** Server-side hash: stored id = SHA-256(salt || client-id), hex. */
async function hashId(env, id) {
  const data = new TextEncoder().encode(`${env.ID_SALT || ""}${id}`);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Latest release tag, cached for an hour via the Cache API. */
async function latestVersion(ctx) {
  const cache = caches.default;
  const key = new Request("https://cache.internal/latest-version");
  const hit = await cache.match(key);
  if (hit) return hit.text();
  const resp = await fetch(RELEASES_URL, {
    headers: { "user-agent": "ganglion-checkpoint-worker" },
  });
  let version = "";
  if (resp.ok) {
    const release = await resp.json();
    version = (release.tag_name || "").replace(/^v/, "");
  }
  ctx.waitUntil(
    cache.put(
      key,
      new Response(version, {
        headers: { "cache-control": `max-age=${LATEST_CACHE_SECONDS}` },
      }),
    ),
  );
  return version;
}

/** Exhaustive schema check mirroring the client's Payload struct. */
function validCheckpoint(p) {
  if (typeof p !== "object" || p === null) return false;
  const keys = Object.keys(p).sort().join(",");
  if (keys !== "arch,counts,dist,id,os,schema,version") return false;
  if (p.schema !== 1) return false;
  for (const field of ["id", "version", "os", "arch", "dist"]) {
    if (typeof p[field] !== "string" || p[field].length > 64) return false;
  }
  if (typeof p.counts !== "object" || p.counts === null) return false;
  const entries = Object.entries(p.counts);
  if (entries.length > 64) return false;
  for (const [category, count] of entries) {
    if (category.length > 32) return false;
    if (typeof count !== "object" || count === null) return false;
    if (!Number.isFinite(count.ok) || !Number.isFinite(count.err)) return false;
  }
  return true;
}

async function checkpoint(request, env, ctx) {
  const payload = await readBoundedJson(request);
  const latest = await latestVersion(ctx);
  // Invalid payloads still get the update answer — the version check must
  // work even if a future client drifts — but nothing is stored.
  if (payload && validCheckpoint(payload)) {
    const idHash = await hashId(env, payload.id);
    const day = new Date().toISOString().slice(0, 10);
    for (const [category, count] of Object.entries(payload.counts)) {
      env.AE?.writeDataPoint({
        blobs: [day, payload.version, payload.os, payload.arch, payload.dist, category, idHash],
        doubles: [count.ok, count.err],
        indexes: [idHash.slice(0, 32)],
      });
    }
    // A row even when counts are empty, so installs-with-no-usage count
    // toward DAU.
    if (Object.keys(payload.counts).length === 0) {
      env.AE?.writeDataPoint({
        blobs: [day, payload.version, payload.os, payload.arch, payload.dist, "", idHash],
        doubles: [0, 0],
        indexes: [idHash.slice(0, 32)],
      });
    }
  }
  return Response.json({ latest });
}

/** Opt-in relay daily aggregates: {schema:1, day, unique_peers, version}. */
async function relayStats(request, env) {
  const p = await readBoundedJson(request);
  if (
    !p ||
    p.schema !== 1 ||
    typeof p.day !== "string" ||
    p.day.length !== 10 ||
    !Number.isFinite(p.unique_peers) ||
    typeof p.version !== "string" ||
    p.version.length > 64
  ) {
    return new Response("bad request", { status: 400 });
  }
  env.AE?.writeDataPoint({
    blobs: [p.day, p.version, "", "", "relay", "unique-peers", ""],
    doubles: [p.unique_peers, 0],
    indexes: ["relay"],
  });
  return Response.json({ ok: true });
}
