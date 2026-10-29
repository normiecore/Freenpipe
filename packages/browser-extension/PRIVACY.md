# Privacy Practices: Handvel Browser Bridge

## Single Purpose
This extension connects your browser to the Handvel desktop app running on your local machine, so Handvel pipes can execute JavaScript in browser tabs.

## Data Usage
- **Host permissions (`<all_urls>`)**: Required to execute JavaScript in any tab the user targets. No data is collected from pages unless explicitly requested by a local Handvel pipe.
- **Tabs permission**: Used to find tabs by URL pattern and get the active tab.
- **Scripting permission**: Used to execute JavaScript code sent from the local Handvel server.
- **Alarms permission**: Used to keep the WebSocket connection alive.

## Data Handling
- All communication is between the extension and `localhost:3030` (the local Handvel server). No data leaves the user's machine.
- The extension does not collect, transmit, or store any user data.
- The extension does not use analytics or tracking.
- The extension does not communicate with any remote servers.

## Remote Code
The extension executes JavaScript code received from `localhost:3030`. This code originates from the user's own Handvel pipes running on their local machine. No code is received from or sent to remote servers.
