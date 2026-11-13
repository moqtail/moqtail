---
'relay': minor
'relay-api': minor
'relay-hooks': minor
'dashboard': minor
---

`GET /api/v1/meta` serves the values under `api.meta` in the relay's configuration as written, without a token, so apps can read things like the relay's public host name from the relay itself. The dashboard shows them on the overview.
