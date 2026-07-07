---
name: music-generation-finite
description: Generate songs, jingles, loops, and instrumental music with Finite-managed FAL MiniMax or ElevenLabs music keys
version: 1.0.0
metadata:
  hermes:
    tags: [audio, music, fal, minimax, elevenlabs, generation, finite]
    related_skills: []
---

# Music Generation

Use this skill when the user asks to create music, a song, a jingle, a theme,
a loop, a backing track, or an instrumental bed.

Do not use text-to-speech for music requests. Use a music generation model and
save the generated audio in the current project or a clear output directory.

## Provider Choice

Prefer FAL MiniMax Music for:

- full songs;
- vocals;
- custom lyrics;
- multilingual songs;
- take-based creative iteration.

Use ElevenLabs Music for:

- short instrumental jingles;
- quick loops;
- background beds;
- cases where FAL is unavailable.

## Credentials

Keys may be in the live environment or in `/home/node/.hermes/.env`.
Always source that file before deciding a provider is unavailable:

```bash
set -euo pipefail
set -a
[ -f /home/node/.hermes/.env ] && . /home/node/.hermes/.env
set +a
```

If both `FAL_KEY` and `ELEVENLABS_API_KEY` are missing, explain that music
generation is not configured for this machine.

## FAL MiniMax Music

Model:

- `fal-ai/minimax-music/v2.5`
- API docs: `https://fal.ai/models/fal-ai/minimax-music/v2.5/api`

Use FAL MiniMax for real songs. Write or refine lyrics first, with short
singable lines and structure tags like `[Intro]`, `[Verse]`, `[Pre Chorus]`,
`[Chorus]`, `[Bridge]`, and `[Outro]`.

Install the Python client if needed:

```bash
python -m pip install fal-client
```

Generate:

```python
import pathlib
import urllib.request

import fal_client

style_prompt = """
Modern pop song, warm synth bass, cinematic drums, memorable chorus,
clear lead vocal, polished production, 95 BPM.
""".strip()

lyrics = """
[Verse]
Write compact singable lyrics here.

[Chorus]
Repeat the hook clearly here.
""".strip()

result = fal_client.subscribe(
    "fal-ai/minimax-music/v2.5",
    arguments={
        "prompt": style_prompt,
        "lyrics": lyrics,
        "is_instrumental": False,
        "lyrics_optimizer": False,
    },
    with_logs=True,
)

audio_url = result["audio"]["url"]
output = pathlib.Path("minimax-song.mp3")
urllib.request.urlretrieve(audio_url, output)
print(output)
```

For instrumental output, omit lyrics and set `is_instrumental: True`.

## ElevenLabs Music

Endpoint:

- `POST https://api.elevenlabs.io/v1/music`
- API docs: `https://elevenlabs.io/docs/api-reference/music/compose`

Quick instrumental generation:

```bash
: "${ELEVENLABS_API_KEY:?ELEVENLABS_API_KEY is not configured}"

prompt='A short upbeat instrumental jingle for an AI agent workshop, warm synths, no vocals'
length_ms=15000
output='elevenlabs-music.mp3'

curl -fsS -X POST "https://api.elevenlabs.io/v1/music" \
  -H "xi-api-key: ${ELEVENLABS_API_KEY}" \
  -H "Content-Type: application/json" \
  -H "Accept: audio/mpeg" \
  --data "$(jq -cn \
    --arg prompt "$prompt" \
    --argjson length_ms "$length_ms" \
    '{prompt: $prompt, music_length_ms: $length_ms, force_instrumental: true}')" \
  -o "$output"

file "$output"
```

For more structured ElevenLabs pieces, call `/v1/music/plan` first and then
pass the returned `composition_plan` to `/v1/music`.

## Workflow

1. Ask one short clarifying question only if the prompt is too vague to choose
   genre, mood, or vocal/instrumental direction.
2. For songs, draft lyrics and a separate style prompt before generation.
3. Start with 10-30 second drafts unless the user asks for a full-length song.
4. Run multiple takes when budget allows; music quality is often take-based.
5. Download the returned audio to a durable project path and verify with
   `file` or `ffprobe`.
6. Deliver the generated media file directly when the chat platform supports
   native attachments. Otherwise, provide the local path.

## Pitfalls

- Do not claim music was generated unless an audio file exists and has been
  verified.
- Do not use public hosting just to pass files around unless the user asks for
  public exposure.
- Avoid artist-name prompts; describe genre, instrumentation, voice, mood,
  production, tempo, and song structure instead.
- If FAL returns an exhausted-balance or invalid-key error, try ElevenLabs if
  appropriate; otherwise leave the lyrics, prompt, and runnable script behind.
