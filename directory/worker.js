// LP2 Online server directory (Cloudflare Worker + one Durable Object, free tier).
// lp2-master servers POST /announce {port, name} every 60 s; games GET /servers.
// The list lives in memory only: if the object restarts, servers reappear on their next announce.
import { DurableObject } from "cloudflare:workers";
import { connect } from "cloudflare:sockets";

const TTL_MS = 150_000; // a server missing two announces drops off
const MAX_SERVERS = 500;

export class Directory extends DurableObject {
  servers = new Map(); // "ip:port" -> { name, seen }

  async fetch(req) {
    const url = new URL(req.url);
    const now = Date.now();
    for (const [k, v] of this.servers) if (now - v.seen > TTL_MS) this.servers.delete(k);

    if (req.method === "GET" && url.pathname === "/servers") {
      return Response.json([...this.servers].map(([addr, v]) => ({ addr, name: v.name })));
    }
    if (req.method === "POST" && url.pathname === "/announce") {
      // The address is the caller's own, so nobody can list someone else's machine.
      // Cloudflare always sets it; only `wrangler dev` on this PC leaves it out or uses ::1.
      let ip = req.headers.get("CF-Connecting-IP") || "127.0.0.1";
      if (ip === "::1") ip = "127.0.0.1";
      if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(ip)) return text(400, "announce over IPv4");
      const body = await req.json().catch(() => null);
      const port = Number(body?.port);
      if (!Number.isInteger(port) || port < 1 || port > 65535) return text(400, "bad port");
      const name = String(body?.name ?? "").replace(/[\x00-\x1f]/g, "").slice(0, 40);
      const addr = `${ip}:${port}`;
      if (!this.servers.has(addr)) {
        if (this.servers.size >= MAX_SERVERS) return text(503, "directory full");
        if (!(await answers(ip, port))) {
          return text(409, `${addr} did not answer on TCP: forward port ${port} (TCP and UDP) to this PC and allow lp2-master through the firewall`);
        }
      }
      this.servers.set(addr, { name, seen: now });
      return Response.json({ addr });
    }
    return text(404, "LP2 Online directory: GET /servers, POST /announce");
  }
}

/** True if an lp2-master answers a List request at ip:port within 3 s. */
async function answers(ip, port) {
  try {
    const sock = connect({ hostname: ip, port });
    const writer = sock.writable.getWriter();
    await writer.write(new TextEncoder().encode('{"op":"List"}\n'));
    const reply = await Promise.race([sock.readable.getReader().read(), new Promise((r) => setTimeout(() => r(null), 3000))]);
    sock.close().catch(() => {});
    return !!reply?.value && new TextDecoder().decode(reply.value).includes('"op"');
  } catch {
    return false;
  }
}

const text = (status, msg) => new Response(msg, { status });

export default {
  fetch(req, env) {
    return env.DIRECTORY.get(env.DIRECTORY.idFromName("main")).fetch(req);
  },
};
