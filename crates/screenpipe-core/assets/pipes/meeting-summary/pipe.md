---
schedule: manual
enabled: true
trigger:
  events:
    - meeting_ended
template: true
title: Meeting Summary
description: When a meeting ends, writes a short summary of it and saves it into the meeting's note (and names untitled meetings).
icon: "🤝"
featured: false
---

A meeting has just finished. Work out what was discussed, write a compact summary, and store it in that meeting's note through the summary endpoint. The user reads it the next time they open the meeting.

This prompt and `buildMeetingSummarizeInstructions` in `apps/screenpipe-app-tauri/lib/utils/meeting-context.ts` (the in-app "summarize with AI" action) describe the same job; change them together.

In the commands below, `<API>` is the Screenpipe API address given in your instructions above (normally `http://localhost:3030`). Send the `Authorization` header shown on every request.

**1. Identify the meeting.** If you were already told which meeting to summarize, read that one with `<API>/meetings/<ID>` and go on to step 2. Otherwise list the latest meetings:

    curl -s -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "<API>/meetings?limit=5"

Rows come newest first. Use the first row whose `meeting_end` is set; a newer meeting may already be open. Keep its `id`, `title`, `meeting_app`, `attendees`, `meeting_start` and `meeting_end`.

**2. Collect what was said.**

    curl -s -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "<API>/meetings/<ID>/transcript"

Each row has `transcript`, `speakerName`, `capturedAt` and `source`, already in time order. When the list is empty, search audio in the meeting's time range instead (`/search?content_type=audio&start_time=<meeting_start>&end_time=<meeting_end>`; audio text is in `content.transcription`). Screen content from the same window can add context such as a shared document or ticket, but the conversation comes first.

**3. Write the summary** in markdown, in the language the meeting was held in:

- two or three sentences on what the meeting was for and how it ended,
- **Decisions**: a bullet list, when decisions were made,
- **Action items**: a bullet list with the owner and due date when they were said,
- **Open questions**: only when something was left unresolved.

Stick to what the transcript supports: no invented names, numbers or promises. For sub-headings use bold text or `###`, never `#` or `##`.

**4. Show it.** Before saving, print the whole section in your reply, starting with the line `## Summary`.

**5. Save it.**

    curl -s -X POST "<API>/meetings/<ID>/summary" \
      -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
      -H "Content-Type: application/json" \
      --data-binary @body.json

`body.json` is `{"summary": "<the markdown from step 3>", "title": "<optional>"}`. Produce it with a JSON encoder (for example a one-line bun script) so quotes and line breaks are escaped. Send only the summary, not the existing note: the server replaces an earlier `## Summary` section or adds one at the end, and keeps everything else the user wrote.

Add `title` only when the current title is empty, generic ("Untitled", "Meeting", just the app name) or plainly wrong. Then give a plain descriptive title of three to eight words, without quotes. Otherwise leave the field out so a title the user picked stays.

A `400` reply means the text was empty or read like an error message rather than a summary. Fix the text and try once more.

**6. Nothing to summarize?** When no speech was captured or the audio is only noise, say so in your reply and stop. Do not save a placeholder.
