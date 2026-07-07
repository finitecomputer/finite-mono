---
name: publish-web-apps-finite
description: Publish a web app from a Finite runtime onto a hostname with private-by-default access, using finitec publish expose/list/remove.
tags: [publishing, web-apps, finitec, runtime]
---

# Publish Web Apps

Use this skill when a human asks you to expose a web app from this machine on a hostname.

Read `/platform/FINITE.md` first. The important contract is:
- do not edit Traefik yourself
- do not invent ad hoc host port mappings
- keep published endpoints private by default
- public exposure requires a single explicit human message containing `MAKE PUBLIC`

## Preconditions

- The app is already running inside the pod on a known port, or you know the command and working directory needed to start it.
- You know the desired auth policy.
- If the user did not explicitly ask for `public`, default to `self`.

## Workflow

1. Get the app running locally, or decide on the command that should run it.
   If you started a temporary preview server for QA on the same port you plan to publish, stop that preview process before using `finitec publish expose --run ...`. `finitec` needs to own the long-lived process for that port.

2. Reserve a hostname from the host:

```bash
finitec publish reserve --label blog
```

3. Publish the app on the chosen port:

```bash
finitec publish expose --hostname HOSTNAME --port 3000 --mode self
```

If the machine should also remember how to start that app again after a runtime restart, include the run command and working directory:

```bash
finitec publish expose --hostname HOSTNAME --port 3000 --run "npm run dev" --cwd /home/node/workspace/blog --mode self
```

When using `--run`, do not leave a separate `nohup`, `tmux`, or background preview server bound to the same port. Stop the preview server first, then let `finitec` start the durable process.
If the app uses Vite, make sure the config also allows the published hostname, not just `0.0.0.0`. `finitec publish` provides `FC_PUBLISH_HOSTNAME` and `FC_PUBLISH_PORT` to the durable app process so Vite can set both `server.allowedHosts` and `preview.allowedHosts` correctly.
The platform-owned publish contract is mirrored at `$FC_PROFILE_ASSETS_ROOT/contracts/publish-runtime-contract.json` if you need the exact env names or bind expectations.

Other supported auth modes:
- `--mode emails --email person@example.com --email other@example.com`
- `--mode org --org-domain finite.vip`
- `--mode public --confirm-public "MAKE PUBLIC"`

4. Verify what is currently published:

```bash
finitec publish list
```

5. Remove an endpoint if needed:

```bash
finitec publish remove --hostname HOSTNAME
```

## Guardrails

- Never choose `public` unless the human explicitly asked for it.
- Never use `--mode public` unless the human has already sent one standalone message containing exactly `MAKE PUBLIC`.
- Do not treat earlier discussion, paraphrases, or vague approval as equivalent to `MAKE PUBLIC`.
- If the human asks for `public`, restate the risk plainly and ask for a standalone `MAKE PUBLIC` message before you do it.
- Prefer the `--run` and `--cwd` form when the app should be restored automatically after the machine restarts.
- Use `finitec` for platform-specific shell commands. It is the supported wrapper on `PATH`.
