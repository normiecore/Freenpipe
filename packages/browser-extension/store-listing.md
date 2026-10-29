# Chrome Web Store Listing

## Name
Handvel Browser Bridge

## Short Description (132 chars max)
Lets the local Handvel app run your own pipes in browser tabs you are signed in to. Everything stays on your machine.

## Detailed Description
Handvel Browser Bridge links your browser to the Handvel desktop app on the same computer, so Handvel pipes can read from and act on pages where you are already signed in.

HOW IT WORKS
The extension opens a WebSocket to the Handvel server on localhost (port 3030 by default). When one of your pipes needs something from a web page, the server passes the request to the extension, which runs it in a matching tab and sends the result back the same way.

USE CASES
- Import your ChatGPT and Claude conversation history into Handvel memories
- Read data from dashboards and internal tools that need a login
- Run web steps as part of a pipe

PRIVACY AND SECURITY
- Traffic goes only between the extension and localhost; nothing is sent to remote servers
- The extension does nothing unless the local Handvel server is running
- Code runs in a tab only when one of your local pipes asks for it
- No tracking, no analytics, no data collection
- Source code (MIT): https://github.com/normiecore/Handvel

REQUIREMENTS
- The Handvel desktop app running on the same machine
- Chrome, Arc, Brave, Edge or another Chromium-based browser

## Category
Productivity

## Language
English

## Privacy Policy URL
https://github.com/normiecore/Handvel/blob/main/packages/browser-extension/PRIVACY.md

## Website
https://github.com/normiecore/Handvel

## Support URL
https://github.com/normiecore/Handvel/issues
