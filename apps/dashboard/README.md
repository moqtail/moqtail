# MOQtail relay dashboard

A web page for watching and operating moqtail relays through their HTTP API
(`/api/v1`). It is a static site: build it once and serve `dist/` from any web
server.

```sh
pnpm install
pnpm build      # writes dist/
pnpm dev        # serves on http://localhost:5173
```

The relay's API must allow the dashboard's origin in `api.cors_origins`, unless
both are served from the same origin.

## Per-host settings: `appsettings.js`

`appsettings.js`, next to `index.html`, sets `window.appSettings` before the
dashboard starts, as in the meet app. The same build can then be deployed on
every machine, each with its own copy of the file. The one in `public/` sets
nothing: the dashboard then keeps its relay list in the browser and starts with
`http://127.0.0.1:4480`.

To show one relay only, set `relay`. The relay menu and the relay list in
Settings are hidden:

```js
window.appSettings = {
  relay: {
    name: 'relay-eu-1',
    baseUrl: 'https://relay-eu-1.example:4480',
    moqUrl: 'https://relay-eu-1.example:4433',
  },
};
```

To offer a list the user can still change, set `relays` to an array of the same
objects. The list is used until the user edits the relays in Settings.

- `baseUrl` is the relay's HTTP API (its `api.bind`). An empty string means the
  dashboard's own origin, for a web server that forwards `/api/v1` to the relay;
  one file then fits every host.
- `moqUrl` is optional: the relay's MoQ address, for its metrics tracks.
- `name` is optional and defaults to the address.

Do not put tokens in `appsettings.js`: anyone who can open the dashboard can
read it. Tokens are entered in Settings and kept in the browser.
