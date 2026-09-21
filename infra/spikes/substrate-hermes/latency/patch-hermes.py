"""Apply narrowly scoped timing/latency seams to exact pinned Hermes sources."""
import argparse
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('source',type=Path);p.add_argument('output',type=Path);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
def patch(rel,edits):
 s=(a.source/rel).read_text()
 for old,new in edits:
  assert s.count(old)==1,(rel,old[:80],s.count(old))
  s=s.replace(old,new)
 (a.output/Path(rel).name).write_text(s)
patch('plugins/platforms/simplex/adapter.py',[
 ('        self._health_task = asyncio.create_task(self._health_monitor())',
  '        self._health_task = asyncio.create_task(self._health_monitor())\n        from wake_control import watch\n        from stream_delivery import install\n        install(self)\n        self._spike_wake_task = asyncio.create_task(watch(self))'),

 ('        resp_type = resp.get("type") or event.get("type", "")',
  '        resp_type = resp.get("type") or event.get("type", "")\n        from latency_trace import trace\n        if resp_type: trace("simplex_event", kind=resp_type)'),
 ('        # Text content\n','        from latency_trace import trace\n        trace("simplex_received", item=meta.get("itemId"))\n\n        # Text content\n'),
 ('            await asyncio.sleep(self._text_batch_delay)','            from latency_trace import mode, trace\n            await asyncio.sleep(mode().get("batch_delay", self._text_batch_delay))\n            trace("batch_flushed")'),
 ('        _voice_exts = {','        from latency_trace import trace\n        trace("simplex_send")\n        _voice_exts = {'),
])
patch('gateway/run.py',[
 ('        _want_stream_deltas = _streaming_enabled',
  '        from latency_trace import mode\n        if mode().get("streaming") and platform_key == "simplex":\n            import copy\n            _scfg = copy.copy(_scfg)\n            _scfg.enabled = True\n            _scfg.transport = "edit"\n            _scfg.edit_interval = .25\n            _scfg.buffer_threshold = 24\n            _streaming_enabled = True\n        _want_stream_deltas = _streaming_enabled'),
 ('        agent.stream_delta_callback = _stream_delta_cb',
  '        if _stream_delta_cb is not None:\n            _original_delta_cb = _stream_delta_cb\n            _first_text_seen = [False]\n            def _timed_delta(text):\n                if text and not _first_text_seen[0]:\n                    from latency_trace import trace\n                    trace("model_first_text")\n                    _first_text_seen[0] = True\n                _original_delta_cb(text)\n            _stream_delta_cb = _timed_delta\n        agent.stream_delta_callback = _stream_delta_cb'),

 ('            result = agent.run_conversation(_api_run_message, **_conversation_kwargs)',
  '            from latency_trace import trace\n            trace("model_turn_start")\n            result = agent.run_conversation(_api_run_message, **_conversation_kwargs)\n            trace("model_turn_end")'),
])
patch('run_agent.py',[
 ('        return interruptible_api_call(self, api_kwargs)',
  '        from latency_trace import trace\n        trace("model_request")\n        result = interruptible_api_call(self, api_kwargs)\n        trace("model_response")\n        return result'),
 ('        if isinstance(text, str) and text:\n            self._current_streamed_assistant_text = (',
  '        if isinstance(text, str) and text:\n            if not getattr(self, "_current_streamed_assistant_text", ""):\n                from latency_trace import trace\n                trace("model_first_text")\n            self._current_streamed_assistant_text = ('),
])

patch('hermes_cli/web_server.py',[
 ('async def get_status(profile: Optional[str] = None):',
  'async def get_status(request: Request = None, profile: Optional[str] = None):\n    from wake_control import request_wake\n    await request_wake(request)'),
])
