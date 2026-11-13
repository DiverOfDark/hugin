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

A widget that already shows values keeps them, dimmed, through two failed refreshes in a row. The error replaces them on the third.
