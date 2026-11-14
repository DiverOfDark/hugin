# Widgets

A widget is a Lua script that fetches a few numbers from a service's API for its card. Scripts live in the cluster as `Widget` resources (`widgets.hugin.dev`). hugin has none built in: the generator writes them, or you do.

A service gets a widget when its Ingress has a `widget.type`:

```yaml
hugin.dev/widget.type: radarr
hugin.dev/widget.url: http://radarr.media.svc.cluster.local
hugin.dev/widget.key-from-secret: plex-secret/RADARR_API_KEY
```

| Annotation | Meaning |
| --- | --- |
| `widget.type` | What the service is. The application's name, such as `radarr`, any name of your own, or `auto`. |
| `widget.url` | Base URL of the service's API. Scripts can call this host only. |
| `widget.hint` | Plain words for the generator: what you want to see, or how the API is reached. |
| `widget.<field>` | Other parameters, passed to the script as `widget.params.<field>`. |
| `hugin.dev/widget.<field>-from-secret` | A credential, see [Credentials from Secrets](credentials.md). |

For a service, hugin uses the first of:

1. A `Widget` named like the `widget.type`, in the service's namespace.
2. A `Widget` with that name in hugin's namespace.
3. The `Widget` the generator wrote for the service.

Until one exists the card says it is waiting, or why generation failed.

A widget shows at most six values. When an upstream call fails, the card shows "API Error" with a short reason such as `HTTP 401` or `timeout`. The reason never includes the URL, because some APIs carry the key in the query string.

## Values that report a problem

A script can mark a value as a warning or an error. The value turns amber or red, the card gets an edge in that colour, and the line at the top of the page names the service.

- **Amber** means "check this when you have time": something is degraded or has failed, but the service still does its job and nothing is lost if it waits a day. A Ceph health of `WARN`, failed downloads, a scrape target that is down, alerts that fire.
- **Red** means "act now": the service cannot do its job, or data can be lost. A Ceph health of `ERR`, a sealed vault, storage with no redundancy left, a full disk, a failed backup, a critical alert.

The generator takes the application's own severity where it has one and picks the lower level when unsure. It leaves normal states alone: a standby node, paused or skipped items, available updates, a long queue. Use `widget.hint` when it marks too much or too little, for example "one firing alert is normal here".

## Stale values

A widget that already shows values keeps them, dimmed, through two failed refreshes in a row. The error replaces them on the third.
