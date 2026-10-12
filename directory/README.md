# LP2 Online server directory

Servers (`lp2-master`) announce themselves here every minute; games ask it for the list of servers
and then search each one for lobbies. It keeps the list in memory only (a Durable Object), checks
that a new server's port answers from the internet before listing it, and lists each server under
the IP it announced from, so nobody can add someone else's machine.

Runs on Cloudflare's free plan.

## Deploy

1. Make a free account at https://dash.cloudflare.com/sign-up.
2. In this folder (needs Node.js):

   ```bash
   npx wrangler login
   ```

   ```bash
   npx wrangler deploy
   ```

3. It prints the address, like `https://lp2-directory.<your-name>.workers.dev` (the public one
   is `https://lp2-directory.lp2rewrite.workers.dev`). For your own, put it in
   `DIRECTORY_URL` (`crates/proto/src/lib.rs`) and rebuild, or set `directory =` in `lp2net.ini`
   and pass `--directory` to `lp2-master`.

Check it: `https://lp2-directory.<your-name>.workers.dev/servers` shows the servers online.

## Test locally

```bash
npx wrangler dev --port 8787
```

Then `lp2-master --directory http://127.0.0.1:8787` and `directory = http://127.0.0.1:8787` in `lp2net.ini`.
