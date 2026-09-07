// Dev-harness transport only. Production routes still use the Hosted Web Device.
import { spawn, execFile, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import path from "node:path";
import { promisify } from "node:util";

const exec = promisify(execFile);

export class NativeSimplex {
  private child: ChildProcess | null = null;
  private queue: Promise<unknown> = Promise.resolve();
  private readonly helper: string;
  constructor(repoRoot: string, private readonly python: string) {
    this.helper = path.join(repoRoot, "finite-agentd/simplex_local.py");
  }

  command(request: Record<string, unknown>): Promise<unknown> {
    const operation = this.queue.then(() => this.dispatch(request));
    this.queue = operation.catch(() => undefined);
    return operation;
  }

  private async call(operation: string, code?: string) {
    try {
      const { stdout } = await exec(this.python, [this.helper, operation, ...(code ? [code] : [])], {
        env: process.env, timeout: 30_000, maxBuffer: 1024 * 1024,
      });
      return JSON.parse(stdout);
    } catch (error) {
      const stdout = (error as { stdout?: string }).stdout;
      if (stdout) {
        try {
          const result = JSON.parse(stdout);
          if (typeof result.error === "string") throw new Error(result.error);
        } catch (parsed) {
          if (parsed instanceof Error && !(parsed instanceof SyntaxError)) throw parsed;
        }
      }
      throw new Error("Local SimpleX operation failed. Check the native trial log.");
    }
  }

  private async dispatch(request: Record<string, unknown>) {
    switch (request.command) {
      case "agent.owner.claim": return {}; // The harness supplies its local test account.
      case "agent.connections.status": return this.call("dashboard-status");
      case "agent.simplex.connect": {
        if (!this.child || this.child.exitCode !== null || this.child.signalCode !== null) {
          this.child = spawn(this.python, [this.helper, "run"], { env: process.env, stdio: "inherit" });
          this.child.on("error", error => console.error("Native SimpleX failed to start", error.message));
        }
        for (let attempt = 0; attempt < 35; attempt++) {
          await new Promise(resolve => setTimeout(resolve, 1000));
          if (this.child.exitCode !== null || this.child.signalCode !== null) {
            throw new Error("Native Hermes stopped. Check ~/.finite-simplex-test/gateway.log.");
          }
          const result = await this.call("dashboard-status");
          if (result.simplex.ready) return {};
        }
        throw new Error("SimpleX is still starting. Refresh to check again.");
      }
      case "agent.simplex.reset": {
        await this.stop();
        await this.call("reset");
        return {};
      }
      case "agent.simplex.disconnect": {
        await this.stop();
        await this.call("disable");
        return {};
      }
      case "agent.simplex.approve_request": {
        const body = request.body as { request_id?: unknown } | undefined;
        if (typeof body?.request_id !== "string" || !/^[a-f0-9]{16}$/.test(body.request_id)) {
          throw new Error("Invalid SimpleX connection request.");
        }
        return this.call("approve-request", body.request_id);
      }
      case "agent.simplex.approve": {
        const body = request.body as { code?: unknown } | undefined;
        if (typeof body?.code !== "string" || !/^[ABCDEFGHJKLMNPQRSTUVWXYZ23456789]{8}$/.test(body.code)) {
          throw new Error("Enter the eight-character pairing code from SimpleX.");
        }
        return this.call("approve", body.code);
      }
      default: throw new Error("This local trial supports only SimpleX connection controls.");
    }
  }

  async stop() {
    const child = this.child;
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, "exit");
    child.kill("SIGTERM");
    await exited;
    this.child = null;
  }
}
