"""Run with the pinned Hermes Python environment; no network or real credentials.

A cancelled request finishes opening after its replacement emits its first
chunk. Only the replacement may own the writer and deliver the remaining text.
"""
import threading
from types import SimpleNamespace
from unittest.mock import MagicMock
from run_agent import AIAgent

agent = AIAgent(api_key='fixture-only', base_url='https://example.invalid/v1', model='fixture', quiet_mode=True, skip_context_files=True, skip_memory=True)
agent.api_mode = 'chat_completions'
agent._interrupt_requested = False
agent._stream_callback = None
delivered = []
agent.stream_delta_callback = delivered.append
old_opening = threading.Event()
release_old = threading.Event()
old_closed = threading.Event()

def chunk(text, finish=None):
    return SimpleNamespace(choices=[SimpleNamespace(index=0, delta=SimpleNamespace(content=text, tool_calls=None, reasoning_content=None, reasoning=None), finish_reason=finish)], model='fixture', usage=None)

def old_create(**kwargs):
    old_opening.set()
    assert release_old.wait(10)
    return iter([chunk('obsolete', 'stop')])

def new_stream():
    yield chunk('fresh-')
    release_old.set()
    assert old_closed.wait(10)
    yield chunk('complete', 'stop')

old_client = MagicMock()
old_client.chat.completions.create.side_effect = old_create
new_client = MagicMock()
new_client.chat.completions.create.side_effect = lambda **kwargs: new_stream()
agent._create_request_openai_client = MagicMock(side_effect=[old_client, new_client])
agent._abort_request_openai_client = MagicMock()
agent._close_request_openai_client = lambda client, **kwargs: old_closed.set() if client is old_client else None
errors = []
def old_call():
    try:
        agent._interruptible_streaming_api_call({'model': 'fixture', 'messages': []})
    except InterruptedError:
        pass
    except Exception as exc:
        errors.append(exc)

thread = threading.Thread(target=old_call)
thread.start()
assert old_opening.wait(10)
agent._interrupt_requested = True
thread.join(10)
assert not thread.is_alive(), 'Interrupt did not release caller'
agent._interrupt_requested = False
try:
    response = agent._interruptible_streaming_api_call({'model': 'fixture', 'messages': []})
finally:
    release_old.set()
assert not errors, errors
assert ''.join(delivered) == 'fresh-complete', delivered
assert response.choices[0].message.content == 'fresh-complete', response
print('PASS: cancelled late opener cannot supersede replacement stream')
