"""Exhaustive ordering model of a container-wide stop's freeze/kill/thaw.

This is a model, not a qualification of zbus, systemd or the kata-agent. It
explores every interleaving (breadth first, so counterexamples are shortest)
of one stop's freezer operations against a transcription of systemd v255's
freezer handling and the cgroup v2 freezer:

- FreezeUnit while FREEZING or THAWING fails (EALREADY); otherwise it writes
  cgroup.freeze=1, cancels any pending freezer call with FreezeCancelled and
  parks its reply until cgroup.events reports frozen (unit.c
  unit_freezer_action, cgroup.c unit_cgroup_freezer_action, dbus-unit.c
  bus_unit_method_freezer_generic).
- ThawUnit writes cgroup.freeze=0, cancels a pending FreezeUnit and replies at
  once when the kernel was not frozen. systemd has no generation: a FreezeUnit
  processed after a ThawUnit freezes again.
- The kernel freezes asynchronously and never while a task waits on a
  submitted FUSE/virtiofs request; unfreezing a leaf cgroup clears frozen
  synchronously (kernel/cgroup/freezer.c cgroup_do_freeze).
- Messages from different D-Bus connections may reach systemd in either
  order. Per-connection FIFO is a model flag, because it is an assumption
  here, not a verified dbus-daemon guarantee.

Designs:
- handler-scoped: Kata 3.29.0 do_signal_process, also the block_in_place
  demonstration (same ordering; it only moves the blocking call off the
  worker). The ttrpc server timeout drops the handler at a sandbox-lock await
  that is Pending after the deadline.
- detached-timeout-thaw: freeze on a spawn_blocking thread bounded by a local
  timeout, then ThawUnit on a new connection (the rejected prototype).
- owner-ordered-thaw: a generation owner that sends ThawUnit on the freeze's
  own connection at the deadline and finishes on the thaw reply.
- owner-reconciled: a generation owner that never submits after its deadline,
  thaws only a freeze observed accepted (cgroup.freeze=1) or answered,
  finishes only when every submitted call has a reply and the cgroup is
  verified thawed, and fences successor admission until then. A thaw error
  leaves it unresolved, never finished.

Violations: late-op (a freezer call processed after its generation finished),
successor-touched, stranded-freeze (quiescent with cgroup.freeze=1 and no live
owner), dishonest-success (success reported while frozen or without kill-all),
false-terminal (finished while a submitted call has no reply), and
not-terminal (quiescent, systemd alive and request released, yet the owner
never finished).

Unimplemented proof obligations: answer() delivers replies instantly. Lost or
delayed replies and retained helper lifetime are absent; M8 (finish without
the freeze reply) survived the retained mutation run and is unqualified.
killall is a counter, not partial signalling, signal failure or task reaping.
The 128-scenario matrix uses release=True; it is not a termination guarantee
for a never-released task. Sole-freezer identity and successor admission are
assumptions, not a runtime owner or crash-recovery implementation. No policy
for retries, escalation, admission or production deadlines is selected here.
"""

from __future__ import annotations

import json
import sys
from collections import deque
from dataclasses import asdict, dataclass
from typing import Any, Callable, Iterator

DESIGNS = ('handler-scoped', 'detached-timeout-thaw', 'owner-ordered-thaw', 'owner-reconciled')
Step = tuple[str, dict[str, Any], tuple[str, ...]]


@dataclass(frozen=True)
class Scenario:
    stuck: bool = True  # a task waits on a submitted FUSE read
    release: bool = True  # the held request is eventually answered
    lock_waiter: bool = False  # another sandbox-lock RPC is queued
    getpids_fail: bool = False
    thaw_error: bool = False  # systemd answers ThawUnit with an error
    systemd_alive: bool = True
    fifo: bool = True  # per-connection D-Bus delivery order
    reuse: bool = False  # a successor with the same unit is admitted when allowed


def initial(design: str, sc: Scenario) -> dict[str, Any]:
    actors = {'handler-scoped': 'H', 'detached-timeout-thaw': 'H'}.get(design, 'H,O')
    return {'kf': 0, 'kz': 0, 'stuck': sc.stuck, 'unit': 'RUNNING', 'pending': None, 'inbox': (),
            'inotify': False, 'queues': (), 'msgs': (), 'replies': (), 'deadline': False, 'killall': 0,
            'terminal': None, 'host': None, 'successor': False, 'conns': 0, 'vars': (),
            'pcs': tuple((a, 0) for a in actors.split(','))}


# -- immutable-state helpers ------------------------------------------------
def key(s: dict[str, Any]) -> tuple:
    return tuple(sorted(s.items()))


def get(s: dict[str, Any], table: str, name: Any, default: Any = None) -> Any:
    return dict(s[table]).get(name, default)


def put(s: dict[str, Any], table: str, name: Any, value: Any) -> dict[str, Any]:
    entries = dict(s[table])
    if value is None:
        entries.pop(name, None)
    else:
        entries[name] = value
    return {**s, table: tuple(sorted(entries.items(), key=lambda kv: str(kv[0])))}


def pc(s: dict[str, Any], actor: str) -> Any:
    return get(s, 'pcs', actor)


def goto(s: dict[str, Any], actor: str, where: Any) -> dict[str, Any]:
    return put(s, 'pcs', actor, where)


def reply(s: dict[str, Any], msg: int) -> str | None:
    return get(s, 'replies', msg)


def new_conn(s: dict[str, Any]) -> tuple[dict[str, Any], str]:
    return {**s, 'conns': s['conns'] + 1}, f"c{s['conns'] + 1}"


def send(s: dict[str, Any], conn: str, op: str) -> tuple[dict[str, Any], int]:
    msg = len(s['msgs']) + 1
    s = {**s, 'msgs': s['msgs'] + ((msg, op),)}
    return put(s, 'queues', conn, get(s, 'queues', conn, ()) + (msg,)), msg


def op_of(s: dict[str, Any], msg: int) -> str:
    return dict(s['msgs'])[msg]


def unanswered(s: dict[str, Any]) -> list[int]:
    return [m for m, _ in s['msgs'] if reply(s, m) is None]


def finish(s: dict[str, Any], outcome: str, host: str) -> tuple[dict[str, Any], tuple[str, ...]]:
    found: list[str] = []
    if unanswered(s):
        found.append('false-terminal')
    if host == 'ok' and (s['kf'] == 1 or s['killall'] == 0):
        found.append('dishonest-success')
    s = {**s, 'terminal': outcome}
    if s['host'] is None:
        s = {**s, 'host': host}
    return s, tuple(found)


# -- environment: kernel, D-Bus delivery, systemd v255 -----------------------
def answer(s: dict[str, Any], msg: int | None, result: str) -> dict[str, Any]:
    return s if msg is None else put(s, 'replies', msg, result)


def systemd_process(s: dict[str, Any], sc: Scenario) -> tuple[dict[str, Any], tuple[str, ...]]:
    msg, s = s['inbox'][0], {**s, 'inbox': s['inbox'][1:]}
    op = op_of(s, msg)
    found = ('late-op',) if s['terminal'] is not None else ()
    if s['successor']:
        found += ('successor-touched',)
    if op == 'freeze':
        if s['unit'] in ('FREEZING', 'THAWING'):
            return answer(s, msg, 'ealready'), ()
        s = answer(s, s['pending'], 'cancelled')
        if s['kz'] == 1:
            return answer({**s, 'unit': 'FROZEN', 'pending': None}, msg, 'ok'), found
        return {**s, 'unit': 'FREEZING', 'kf': 1, 'pending': msg}, found
    if sc.thaw_error:
        return answer(s, msg, 'error'), ()
    if s['unit'] == 'THAWING':
        return answer(s, msg, 'ealready'), ()
    was_frozen = s['kz']
    s = answer({**s, 'kf': 0, 'kz': 0, 'inotify': s['inotify'] or was_frozen == 1}, s['pending'], 'cancelled')
    if was_frozen == 0:
        return answer({**s, 'unit': 'RUNNING', 'pending': None}, msg, 'ok'), found
    return {**s, 'unit': 'THAWING', 'pending': msg}, found


def environment(s: dict[str, Any], sc: Scenario) -> Iterator[Step]:
    if s['stuck'] and sc.release:
        yield 'held request answered', {**s, 'stuck': False}, ()
    if s['kf'] == 1 and s['kz'] == 0 and not s['stuck']:
        yield 'kernel: cgroup frozen', {**s, 'kz': 1, 'inotify': True}, ()
    for conn, queue in s['queues']:
        for index in range(len(queue) if not sc.fifo else min(1, len(queue))):
            rest = queue[:index] + queue[index + 1:]
            moved = put({**s, 'inbox': s['inbox'] + (queue[index],)}, 'queues', conn, rest or None)
            yield f'dbus: deliver m{queue[index]} from {conn}', moved, ()
    if sc.systemd_alive and s['inbox']:
        nxt, found = systemd_process(s, sc)
        yield f"systemd: process m{s['inbox'][0]} {op_of(s, s['inbox'][0])}", nxt, found
    if sc.systemd_alive and s['inotify']:
        nxt = {**s, 'inotify': False}
        if s['unit'] in ('FREEZING', 'THAWING'):
            settled = 'RUNNING' if s['kz'] == 0 else 'FROZEN'
            nxt = answer({**nxt, 'unit': settled, 'pending': None}, s['pending'], 'ok')
        yield 'systemd: cgroup.events', nxt, ()
    if not s['deadline'] and s['terminal'] is None:
        yield 'deadline', {**s, 'deadline': True}, ()
    if sc.reuse and not s['successor'] and successor_allowed(s):
        # A successor with the same unit name; any later call of this
        # generation that systemd processes acts on the successor's unit.
        yield 'admit successor', {**s, 'successor': True}, ()


def successor_allowed(s: dict[str, Any]) -> bool:
    # Unsafe designs admit as soon as the RPC handler is gone; owners fence
    # admission until their generation finished.
    owner = pc(s, 'O')
    return s['terminal'] is not None if owner is not None else s['host'] is not None


# -- designs ------------------------------------------------------------------
def handler_scoped(s: dict[str, Any], sc: Scenario) -> Iterator[Step]:
    where = pc(s, 'H')

    def lock(nxt: Any) -> Iterator[Step]:
        if s['deadline'] and sc.lock_waiter:
            done, found = finish(goto(s, 'H', 'done'), 'dropped', 'deadline')
            yield 'H: dropped at sandbox-lock await', done, found
        else:
            yield 'H: sandbox lock', goto(s, 'H', nxt), ()

    if where == 0:
        yield from lock(1)
    elif where == 1:
        t, conn = new_conn(s)
        t, msg = send(t, conn, 'freeze')
        yield 'H: send FreezeUnit (blocking)', goto(put(t, 'vars', 'freeze', msg), 'H', 2), ()
    elif where == 2 and reply(s, get(s, 'vars', 'freeze')) is not None:
        yield 'H: freeze call returned (errors only logged)', goto(s, 'H', 3), ()
    elif where == 3:
        yield from lock(4)
    elif where == 4:
        if sc.getpids_fail:
            done, found = finish(goto(s, 'H', 'done'), 'error-returned', 'error')
            yield 'H: get_pids failed, return before thaw', done, found
        else:
            yield 'H: kill all pids', goto({**s, 'killall': s['killall'] + 1}, 'H', 5), ()
    elif where == 5:
        yield from lock(6)
    elif where == 6:
        t, conn = new_conn(s)
        t, msg = send(t, conn, 'thaw')
        yield 'H: send ThawUnit (blocking)', goto(put(t, 'vars', 'thaw', msg), 'H', 7), ()
    elif where == 7 and reply(s, get(s, 'vars', 'thaw')) is not None:
        done, found = finish(goto(s, 'H', 'done'), 'completed', 'ok')
        yield 'H: thaw returned (errors only logged), reply ok', done, found


def detached_timeout_thaw(s: dict[str, Any], sc: Scenario) -> Iterator[Step]:
    where, helper = pc(s, 'H'), pc(s, 'F')
    if helper == 0:
        t, conn = new_conn(s)
        yield 'F: zbus connect', goto(put(t, 'vars', 'fconn', conn), 'F', 1), ()
    elif helper == 1:
        t, msg = send(s, get(s, 'vars', 'fconn'), 'freeze')
        yield 'F: send FreezeUnit', goto(put(t, 'vars', 'freeze', msg), 'F', 2), ()
    freeze = get(s, 'vars', 'freeze')
    if where == 0:
        yield 'H: spawn_blocking freeze', goto(goto(s, 'F', 0), 'H', 1), ()
    elif where == 1:
        if freeze is not None and reply(s, freeze) is not None:
            yield 'H: freeze result', goto(s, 'H', 2), ()
        if s['deadline']:
            t, conn = new_conn(s)
            t, msg = send(t, conn, 'thaw')
            yield 'H: local timeout, ThawUnit on a new connection', goto(put(t, 'vars', 'thaw', msg), 'H', 10), ()
    elif where == 2:
        if sc.getpids_fail:
            done, found = finish(goto(s, 'H', 'done'), 'error-returned', 'error')
            yield 'H: get_pids failed', done, found
        else:
            t, conn = new_conn({**s, 'killall': s['killall'] + 1})
            t, msg = send(t, conn, 'thaw')
            yield 'H: kill all, send ThawUnit', goto(put(t, 'vars', 'thaw', msg), 'H', 3), ()
    elif where == 3 and reply(s, get(s, 'vars', 'thaw')) is not None:
        done, found = finish(goto(s, 'H', 'done'), 'completed', 'ok')
        yield 'H: thaw returned, reply ok', done, found
    elif where == 10 and reply(s, get(s, 'vars', 'thaw')) is not None:
        done, found = finish(goto(s, 'H', 'done'), 'aborted', 'error')
        yield 'H: thaw returned, discard freeze, reply error', done, found


def rpc_handler(s: dict[str, Any]) -> Iterator[Step]:
    """The RPC handler only awaits the owner; its deadline never ends the generation."""
    if pc(s, 'H') == 0:
        if s['terminal'] is not None:
            yield 'H: owner finished, reply', goto({**s, 'host': s['host'] or 'owner'}, 'H', 'done'), ()
        elif s['deadline']:
            yield 'H: RPC deadline, owner retained', goto({**s, 'host': 'deadline-owner-retained'}, 'H', 'done'), ()


def owner_ordered_thaw(s: dict[str, Any], sc: Scenario) -> Iterator[Step]:
    yield from rpc_handler(s)
    where, freeze = pc(s, 'O'), get(s, 'vars', 'freeze')
    if where == 0:
        t, conn = new_conn(s)
        t, msg = send(put(t, 'vars', 'oconn', conn), conn, 'freeze')
        yield 'O: send FreezeUnit', goto(put(t, 'vars', 'freeze', msg), 'O', 1), ()
    elif where == 1:
        if reply(s, freeze) == 'ok':
            killed = s if sc.getpids_fail else {**s, 'killall': s['killall'] + 1}
            t, msg = send(killed, get(s, 'vars', 'oconn'), 'thaw')
            yield 'O: frozen, kill all, ThawUnit on the same connection', goto(put(t, 'vars', 'thaw', msg), 'O', 2), ()
        elif s['deadline']:
            t, msg = send(s, get(s, 'vars', 'oconn'), 'thaw')
            yield 'O: deadline, ThawUnit on the same connection', goto(put(t, 'vars', 'thaw', msg), 'O', 2), ()
    elif where == 2 and reply(s, get(s, 'vars', 'thaw')) is not None:
        ok = reply(s, get(s, 'vars', 'thaw')) == 'ok' and s['killall'] > 0
        done, found = finish(goto(s, 'O', 'done'), 'completed' if ok else 'aborted', 'ok' if ok else 'error')
        yield 'O: thaw reply, finished', done, tuple(f for f in found if f != 'false-terminal') + (
            ('false-terminal',) if reply(s, freeze) is None else ())


def owner_reconciled(s: dict[str, Any], sc: Scenario) -> Iterator[Step]:
    yield from rpc_handler(s)
    where, freeze, thaw = pc(s, 'O'), get(s, 'vars', 'freeze'), get(s, 'vars', 'thaw')
    if where == 0:
        if s['deadline']:
            done, found = finish(goto(s, 'O', 'done'), 'aborted-before-submit', 'error')
            yield 'O: deadline before submit, never send FreezeUnit', done, found
        else:
            t, conn = new_conn(s)
            t, msg = send(t, conn, 'freeze')
            yield 'O: submit FreezeUnit', goto(put(t, 'vars', 'freeze', msg), 'O', 1), ()
    elif where == 1:
        answered = reply(s, freeze)
        if answered == 'ok':
            killed = {**s, 'vars': s['vars']} if sc.getpids_fail else {**s, 'killall': s['killall'] + 1}
            label = 'O: frozen, get_pids failed, thaw anyway' if sc.getpids_fail else 'O: frozen, kill all'
            yield label, goto(killed, 'O', 2), ()
        elif answered is not None or (s['deadline'] and s['kf'] == 1):
            yield 'O: freeze answered or observed accepted, cancel by thaw', goto(s, 'O', 2), ()
    elif where == 2:
        t, conn = new_conn(s)
        t, msg = send(t, conn, 'thaw')
        yield 'O: send ThawUnit', goto(put(t, 'vars', 'thaw', msg), 'O', 3), ()
    elif where == 3 and reply(s, thaw) is not None:
        if reply(s, thaw) != 'ok':
            yield 'O: thaw failed, unresolved and fenced', goto(s, 'O', 'unresolved'), ()
        elif reply(s, freeze) is not None and s['kf'] == 0 and s['kz'] == 0:
            ok = reply(s, freeze) == 'ok' and s['killall'] > 0
            done, found = finish(goto(s, 'O', 'done'), 'completed' if ok else 'aborted', 'ok' if ok else 'error')
            yield 'O: all calls answered, cgroup verified thawed, finished', done, found


DESIGN_STEPS: dict[str, Callable[[dict[str, Any], Scenario], Iterator[Step]]] = {
    'handler-scoped': handler_scoped, 'detached-timeout-thaw': detached_timeout_thaw,
    'owner-ordered-thaw': owner_ordered_thaw, 'owner-reconciled': owner_reconciled}


def successors(design: str, s: dict[str, Any], sc: Scenario) -> list[Step]:
    return [*DESIGN_STEPS[design](s, sc), *environment(s, sc)]


def quiescent_violations(design: str, s: dict[str, Any], sc: Scenario) -> tuple[str, ...]:
    owner_live = pc(s, 'O') not in (None, 'done') if design.startswith('owner') else False
    found = []
    if s['kf'] == 1 and not owner_live:
        found.append('stranded-freeze')
    if sc.systemd_alive and not s['stuck'] and design.startswith('owner') and s['terminal'] is None and not sc.thaw_error:
        found.append('not-terminal')
    return tuple(found)


def replay(design: str, sc: Scenario, labels: list[str]) -> dict[str, Any]:
    """Re-applies a counterexample trace step by step; fails on any unknown label."""
    s = initial(design, sc)
    for label in labels:
        if label == 'quiescent':
            if successors(design, s, sc):
                raise ValueError('trace is not quiescent at its end')
            continue
        matches = [nxt for step, nxt, _ in successors(design, s, sc) if step == label]
        if len(matches) != 1:
            raise ValueError(f'step {label!r} enabled {len(matches)} times')
        s = matches[0]
    return s


def explore(design: str, sc: Scenario, limit: int = 200_000) -> dict[str, Any]:
    start = initial(design, sc)
    parent: dict[tuple, tuple[tuple, str] | None] = {key(start): None}
    queue = deque([start])
    traces: dict[str, list[str]] = {}
    quiescent = 0
    outcomes: set[str] = set()

    def trace(k: tuple, last: str) -> list[str]:
        labels = [last]
        while parent[k] is not None:
            k, label = parent[k]
            labels.append(label)
        return labels[::-1]

    while queue:
        s = queue.popleft()
        k = key(s)
        steps = successors(design, s, sc)
        if not steps:
            quiescent += 1
            outcomes.add(f"terminal={s['terminal']} host={s['host']} kf={s['kf']} owner={pc(s, 'O')} successor={s['successor']}")
            for v in quiescent_violations(design, s, sc):
                traces.setdefault(v, trace(k, 'quiescent'))
        for label, nxt, found in steps:
            for v in found:
                traces.setdefault(v, trace(k, label))
            nk = key(nxt)
            if nk not in parent:
                if len(parent) >= limit:
                    raise RuntimeError(f'state limit {limit} reached for {design} {sc}')
                parent[nk] = (k, label)
                queue.append(nxt)
    return {'design': design, 'scenario': asdict(sc), 'states': len(parent), 'quiescent': quiescent,
            'violations': {v: traces[v] for v in sorted(traces)}, 'outcomes': sorted(outcomes)}


def matrix() -> Iterator[Scenario]:
    for stuck in (True, False):
        for lock_waiter in (False, True):
            for getpids_fail in (False, True):
                for thaw_error in (False, True):
                    for systemd_alive in (True, False):
                        for fifo in (True, False):
                            for reuse in (False, True):
                                yield Scenario(stuck=stuck, lock_waiter=lock_waiter, getpids_fail=getpids_fail,
                                               thaw_error=thaw_error, systemd_alive=systemd_alive, fifo=fifo, reuse=reuse)


def main(argv: list[str]) -> int:
    report: dict[str, Any] = {}
    for design in DESIGNS:
        kinds: dict[str, int] = {}
        states = 0
        for sc in matrix():
            result = explore(design, sc)
            states += result['states']
            for v in result['violations']:
                kinds[v] = kinds.get(v, 0) + 1
        report[design] = {'scenarios': 128, 'states': states, 'scenarios_with_violation': kinds}
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
