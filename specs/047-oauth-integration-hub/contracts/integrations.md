# The curated integrations of release 0.3.0 (design 047, Requirement 047-023)

Required for the release (Task 047-12) are the first four; the others are content tasks of the same release (Task
047-22). Presets listed are the first ones; each integration's `review.md` is the final word. Real-provider runs need
accounts and credentials that only the owner can create (Gap 047-006).

| Integration | Provider and hosts | Scopes (required; optional) | First presets |
|---|---|---|---|
| GitHub | `github.com` (authorize, token), `api.github.com` | none; `repo`, `read:org`, `workflow` | `getRepository`, `createIssue`, `commentOnIssue`, `dispatchWorkflow` |
| Google | `accounts.google.com`, `oauth2.googleapis.com`; APIs `sheets.googleapis.com`, `www.googleapis.com` (Drive), `gmail.googleapis.com` | `openid`, `email`; `spreadsheets`, `drive.file`, `gmail.send` | `readRange`, `appendRows`, `uploadFile`, `sendMail` |
| Microsoft Graph | `login.microsoftonline.com/{directory}` (bound from the registration), `graph.microsoft.com` | `openid`, `email`, `offline_access`; `Mail.Send`, `ChannelMessage.Send`, `Files.ReadWrite` | `sendMail`, `postChannelMessage`, `uploadDriveItem` |
| Generic OAuth 2.0 API | the tenant's provider (authored) | as the provider lists | none (a resource type template with `http.request`) |
| Slack | `slack.com` | `chat:write`; `channels:read` | `postMessage` |
| Atlassian (Jira) | `auth.atlassian.com`, `api.atlassian.com` (the cloud identifier is a preset parameter) | `read:jira-work`, `write:jira-work`, `offline_access` | `createIssue`, `transitionIssue` |
| Salesforce | `login.salesforce.com`; `*.my.salesforce.com` bound from `instance_url` | `api`, `refresh_token` | `query`, `createRecord` |
| HubSpot | `app.hubspot.com`, `api.hubapi.com` | `crm.objects.contacts.read`; `crm.objects.contacts.write` | `getContact`, `createContact` |
| Notion | `api.notion.com` | none (Notion grants by page) | `queryDatabase`, `createPage` |
| Zoom | `zoom.us`, `api.zoom.us` | `meeting:write` | `createMeeting` |
