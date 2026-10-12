"""Emit an inert JSON scenario's text and exit code; never run a command."""

import sys
import time

from contract import ContractError, read_json


def main():
    try:
        if len(sys.argv) != 2:
            raise ContractError("expected one synthetic scenario path")
        scenario = read_json(sys.argv[1])
        keys = {"exit_code", "stdout", "stderr", "delay_ms"}
        if type(scenario) is not dict or set(scenario) != keys:
            raise ContractError("scenario: expected exact fake-executor fields")
        code = scenario["exit_code"]
        delay = scenario["delay_ms"]
        if type(code) is not int or not 0 <= code <= 255:
            raise ContractError("scenario.exit_code: expected integer 0..255")
        if type(delay) is not int or not 0 <= delay <= 5000:
            raise ContractError("scenario.delay_ms: expected integer 0..5000")
        for field in ("stdout", "stderr"):
            if type(scenario[field]) is not str:
                raise ContractError(f"scenario.{field}: expected string")
        time.sleep(delay / 1000)
        sys.stdout.buffer.write(scenario["stdout"].encode("utf-8"))
        sys.stdout.buffer.flush()
        sys.stderr.buffer.write(scenario["stderr"].encode("utf-8"))
        sys.stderr.buffer.flush()
        return code
    except (OSError, ContractError, UnicodeError) as exc:
        print(f"fake scenario error: {exc}", file=sys.stderr)
        return 64


if __name__ == "__main__":
    sys.exit(main())
