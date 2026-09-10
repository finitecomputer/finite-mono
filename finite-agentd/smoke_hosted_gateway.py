import asyncio
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from aiohttp import ClientSession, WSMsgType

import gateway_ingress


async def main():
    with tempfile.TemporaryDirectory(prefix="finite-gateway-smoke-") as directory:
        root = Path(directory)
        home = root / "agent"
        (home / "agentd").mkdir(parents=True)
        token = "e" * 64
        (home / "config.json").write_text(json.dumps({"account_id": "f" * 64}))
        (home / "agentd/hosted-gateway.json").write_text(
            json.dumps({"enabled": True, "token": token})
        )
        environment = dict(
            os.environ,
            FINITECHAT_HOME=str(home),
            HERMES_HOME=str(root / "hermes"),
            PATH=str(Path(sys.executable).parent) + os.pathsep + os.environ["PATH"],
        )
        log = (root / "process.log").open("w+")
        process = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("hosted_gateway.py"))],
            env=environment,
            stdout=log,
            stderr=log,
            start_new_session=True,
        )

        class Handler(BaseHTTPRequestHandler):
            rbufsize = 0

            def do_GET(self):
                gateway_ingress.forward(self, home)

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            async with ClientSession() as client:
                for _ in range(90):
                    if process.poll() is not None:
                        raise RuntimeError("Hermes exited during startup")
                    try:
                        async with client.get(
                            "http://127.0.0.1:9120/api/sessions?limit=1",
                            headers={"X-Hermes-Session-Token": token},
                        ) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        pass
                    await asyncio.sleep(1)
                else:
                    raise RuntimeError("Hermes readiness timed out")
                endpoint = f"http://127.0.0.1:{server.server_port}/gateway"
                headers = {
                    "Host": "r-" + ("a" * 20) + ".agents.lat3.finite.computer",
                    "X-Finite-Agent-Account-Id": "f" * 64,
                }
                async with client.get(
                    endpoint + "/api/sessions?limit=1",
                    headers=dict(headers, Authorization="Bearer " + token),
                ) as response:
                    assert response.status == 200, response.status
                async with client.get(endpoint + "/", headers=headers) as response:
                    assert response.status == 401, response.status
                async with client.ws_connect(
                    endpoint + "/api/ws?token=" + token,
                    headers=dict(headers, Origin="http://localhost:3000"),
                ) as ws:
                    # Prove actual pinned protocol handshake and request/response.
                    await ws.send_json(
                        {"jsonrpc": "2.0", "id": 1, "method": "session.list", "params": {}}
                    )
                    for _ in range(20):
                        message = await ws.receive_json(timeout=15)
                        if message.get("id") == 1:
                            assert "error" not in message, str(message.get("error"))
                            break
                    else:
                        raise RuntimeError("No session.list response")
                    (home / "agentd/hosted-gateway.json").write_text(
                        json.dumps({"enabled": False, "token": None})
                    )
                    os.killpg(process.pid, signal.SIGTERM)
                    await asyncio.to_thread(process.wait, timeout=12)
                    closed = await ws.receive(timeout=3)
                    assert closed.type in {WSMsgType.CLOSE, WSMsgType.CLOSED, WSMsgType.CLOSING}, (
                        closed.type
                    )
                print(
                    "Pinned Hermes: authenticated REST and session.list WebSocket round trip pass; anonymous HTML refused"
                )
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            await asyncio.to_thread(server.shutdown)
            server.server_close()
            thread.join()
            log.seek(0)
            output = log.read()
            log.close()
            assert token not in output, "Hermes logged the session token"
            print("Pinned Hermes: no credential in captured process output")


if __name__ == "__main__":
    asyncio.run(main())
