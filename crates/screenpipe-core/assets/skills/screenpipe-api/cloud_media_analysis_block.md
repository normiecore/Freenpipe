<!-- BEGIN_CLOUD_MEDIA_ANALYSIS -->

## Media analysis through the user's AI gateway — `POST /v1/chat/completions`

The user opted in to sending selected media to the AI gateway they configured in Freenpipe. Reach for it when a question needs real understanding of pictures or sound (what a screen recording shows, what was said in a short clip) and the OCR text, transcripts and search results you already have do not answer it.

Send the request to the local server. It forwards the body to the gateway and attaches the gateway credential, which you never see.

```bash
curl -X POST http://localhost:3030/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "<multimodal model served by the gateway>",
    "max_tokens": 400,
    "messages": [{
      "role": "user",
      "content": [
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}},
        {"type": "text",      "text": "what is on this screen?"}
      ]
    }]
  }'
```

### Preparing inputs
- Screen recordings: pull a few keyframes with ffmpeg and attach each as an `image_url` part.
- Audio: attach one short WAV or MP3 clip as an `audio_url` part holding a base64 data URL. Not every gateway or model accepts audio.
- Keep each request small: a handful of images or one short clip.

### Picking the model
The gateway decides which models exist. Use the model the user named; if they named none, ask once which multimodal model to use. When the gateway says the model is unknown or cannot take that kind of input, tell the user rather than guessing other names.

### Errors
- `503 cloud_token_missing`: no gateway is configured any more. Say so and continue with local tools.
- `502 upstream_unreachable`: the gateway did not answer. Retry once, then continue with local tools.

### When not to use it
- Plain text questions: the user's chat model already handles those.
- Requests where the user asked for on-device processing only.

<!-- END_CLOUD_MEDIA_ANALYSIS -->
