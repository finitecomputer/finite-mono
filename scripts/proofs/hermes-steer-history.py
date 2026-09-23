"""Offline regression using pinned Hermes and real scratch SQLite.

Persist a tool result before accepting a correction, then complete the native
tool handoff normally or after Stop. The correction must survive exactly once.
Run with the runtime image's Python; no network or real credentials are needed.
"""
from pathlib import Path
from tempfile import TemporaryDirectory
from hermes_state import SessionDB
from run_agent import AIAgent


def prove(stopped, model=False, consumed=False):
    with TemporaryDirectory() as scratch:
        db = SessionDB(db_path=Path(scratch) / 'state.db')
        db.create_session('cancel-proof', 'cli', model='fixture')
        agent = AIAgent(api_key='fixture-only', base_url='https://example.invalid/v1', model='fixture', session_db=db, session_id='cancel-proof', quiet_mode=True, skip_context_files=True, skip_memory=True)
        messages = [{'role':'user','content':'initial request'}, {'role':'assistant','content':'working','tool_calls':[{'id':'tool-one','type':'function','function':{'name':'terminal','arguments':'{}'}}]}, {'role':'tool','tool_call_id':'tool-one','content':'stopped tool'}]
        if model:
            messages = messages[:1]
        agent._persist_session(messages)
        before = db._conn.execute("select role, content from messages order by id").fetchall()
        if model:
            agent._model_request_active.set()
            assert agent.redirect('accepted correction')
        else:
            assert agent.steer('accepted correction')
        if consumed:
            # The model loop can consume and persist the redirect before the
            # gateway handles Stop. No pending buffer remains for Stop to drain.
            correction = agent._drain_pending_redirect()
            assert correction == 'accepted correction'
            messages.extend([{'role': 'assistant', 'content': 'partial response'},
                             {'role': 'user', 'content': correction}])
            agent._persist_session(messages)
        if stopped:
            agent.hard_interrupt()
        if not model:
            agent._apply_pending_steer_to_tool_results(messages, 1)
        agent._persist_session(messages)
        agent._persist_session(messages)
        assert db._conn.execute("select count(*) from messages where role='user' and content='accepted correction'").fetchone()[0] == 1, 'accepted correction not persisted exactly once'
        assert agent._drain_pending_steer() is None, 'delivered correction must not requeue'
        assert agent._hard_interrupt_requested.is_set() == stopped, 'persistence changed Stop'
        after = db._conn.execute("select role, content from messages order by id").fetchall()
        assert after[:len(before)] == before, "steering rewrote durable history"
        assert agent._drain_pending_redirect() is None, "stopped redirect must not replay"
        if stopped:
            assert tuple(after[-1]) == ('assistant', 'Stopped before applying this message.'), 'Stop must separate the canceled correction from the next user turn'
        print(f'PASS: real SQLite writer persists correction once; stopped={stopped}; model={model}; consumed={consumed}')
        db.close()


prove(False)
prove(True)

prove(True, model=True)

prove(True, model=True, consumed=True)
