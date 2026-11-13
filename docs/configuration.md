# Configuration

The chart renders its `config` value to `/config/config.json`. All keys are optional.

| Key | Meaning | Default |
| --- | --- | --- |
| `title` | Page title. | `Hugin` |
| `columns` | Most group columns on a wide screen. | `5` |
| `showStats` | Show pod CPU and memory on each discovered service. | `true` |
| `favicons` | `fallback` shows a service's own favicon on cards without an `icon`; `prefer` shows it on every card and keeps `icon` for services without one; `off` never fetches. | `fallback` |
| `probes` | Ask the page behind every card whether it answers, see [Services](services.md#the-light-on-a-card). | `true` |
| `groupOrder` | Groups to show first, in this order. The rest follow alphabetically. | `[]` |
| `discovery.ingress` | Read Ingress annotations. | `true` |
| `discovery.traefikIngressRoutes` | Also read annotations on Traefik `IngressRoute` objects. | `false` |
| `discovery.widgets` | Load widget scripts from `Widget` resources. | `true` |
| `header.cluster.show`, `header.cluster.label` | Cluster-wide CPU and memory. | `true`, `cluster` |
| `header.nodes.show` | CPU and memory per node. | `true` |
| `header.weather` | `label`, `latitude`, `longitude`, `timezone`, `units` (`metric` or `imperial`), `cacheMinutes`, `maximumFractionDigits`. | off |
| `header.search` | `provider` (`google`, `duckduckgo`, `bing`, `brave`, or `custom` with `url`), `target`. | off |
| `bookmarks` | List of `{ group, items: [{ name, href, abbr, icon }] }`. | `[]` |
| `services` | Static entries, see [Services](services.md#static-services). | `[]` |
| `intervals.widgetsSeconds` | How often widgets refresh. | `30` |
| `intervals.resourcesSeconds` | How often node and pod usage refresh. | `15` |
| `intervals.idleAfterSeconds` | Polling stops this long after the last page view. | `90` |
| `intervals.requestTimeoutSeconds` | Timeout for one upstream request. | `10` |
| `intervals.probeSeconds` | How often every page is asked whether it answers. | `60` |

Unknown keys are rejected at startup, so a typo does not pass silently.
