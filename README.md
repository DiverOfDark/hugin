# Hugin

Hugin is a small dashboard for a Kubernetes homelab. It lists your services by reading annotations on Ingresses, shows whether their pods are up, and adds a few numbers from each service's own API.

It is one Rust binary that renders HTML on the server and pushes updates to the browser over server-sent events. On a cluster with about 400 pods and 38 services it uses about 10 MiB of memory and 10 millicores while the page is open.

Hugin reads the `gethomepage.dev/*` annotations of [gethomepage](https://gethomepage.dev), so a cluster annotated for gethomepage works without changing any manifest.

## Features

- Service discovery from Ingress annotations, and optionally from Traefik `IngressRoute` objects.
- A status dot per service from pod phase, plus CPU and memory of the pods.
- Widgets are sandboxed Lua scripts. Fourteen are built in: Argo CD, Grafana, Nextcloud, Prometheus, ruTorrent, Suwayomi, Traefik, FRITZ!Box, Pi-hole, Paperless-ngx, Sonarr, Radarr, Jackett and Seerr (Jellyseerr, Overseerr).
- Your own widgets as `Widget` resources in the cluster, with no rebuild.
- An optional generator that writes a widget for any service with a language model and validates it against the live service.
- Widget credentials read from Kubernetes Secrets by reference, with RBAC limited to the Secrets you name.
- Header with cluster and node CPU and memory, Open-Meteo weather and a search box.
- Static entries and bookmarks from configuration.
- Upstream APIs are polled only while someone has the page open.
- `/healthz` and Prometheus `/metrics` on a separate port.

## Install

```bash
helm install hugin oci://ghcr.io/diverofdark/charts/hugin \
  --namespace hugin --create-namespace \
  --set ingress.enabled=true \
  --set 'ingress.hosts[0].host=status.example.com' \
  --set 'ingress.hosts[0].paths[0].path=/' \
  --set 'ingress.hosts[0].paths[0].pathType=Prefix'
```

The chart also lives in this repository under `charts/hugin`, which is what an Argo CD Application can point at.

The dashboard needs [metrics-server](https://github.com/kubernetes-sigs/metrics-server) for CPU and memory figures. Without it the status dots still work.

## Annotations

Put these on an Ingress. Each one can be written with the `gethomepage.dev/` prefix or the `hugin.dev/` prefix. When both are present, `hugin.dev/` wins.

| Annotation | Meaning | Default |
| --- | --- | --- |
| `enabled` | Must be `"true"` for the service to be listed. | not listed |
| `name` | Display name. | the Ingress name |
| `group` | Column the service appears in. | `Kubernetes` |
| `href` | Link target. | the first Ingress host |
| `icon` | Icon reference, see below. | none |
| `description` | Second line under the name. | none |
| `pod-selector` | Label selector for the pods behind the service. | `app.kubernetes.io/name=<app>` |
| `app` | Name used in the default pod selector. | the Ingress name |
| `weight` | Sort order inside the group, then by name. | `0` |
| `widget.type` | Widget to show, see [Widgets](#widgets). | none |
| `widget.url` | Base URL of the service's API. | none |
| `widget.<field>` | Any other widget parameter, such as `key`, `username`, `password`, `fields`, `enableQueue`. | none |

Pods are looked up in the Ingress's own namespace.

The status dot is green when every matching pod is in phase `Running` or `Succeeded`. It is orange when only some are, when none are, or when no pod matches.

### Icons

| Reference | Source |
| --- | --- |
| `https://...` or `/path` | Used as written. |
| `name.png`, `name.svg`, `name.webp`, or `name` | [dashboard-icons](https://github.com/homarr-labs/dashboard-icons) (`name` means png). |
| `mdi-name` | Material Design Icons. |
| `si-name` | Simple Icons. |

Icons are loaded by the browser, not by Hugin.

### Credentials from Secrets

Instead of writing an API key into an annotation, reference a Secret:

```yaml
metadata:
  annotations:
    gethomepage.dev/enabled: "true"
    gethomepage.dev/widget.type: argocd
    gethomepage.dev/widget.url: http://argocd-server.argocd.svc.cluster.local
    hugin.dev/widget.key-from-secret: argocd-readonly-token/token
```

`hugin.dev/widget.<field>-from-secret: <secret-name>/<key>` reads that key of that Secret and uses it as `widget.<field>`. The Secret must be in the same namespace as the Ingress.

Hugin may only read Secrets listed in the chart's `secretAccess` value:

```yaml
secretAccess:
  - namespace: argocd
    names: [argocd-readonly-token]
```

Each entry becomes a Role that allows `get` on exactly those Secrets. Hugin cannot list Secrets or read any other.

A plain `widget.<field>` annotation next to the reference is the fallback: it is used when the Secret cannot be read. This lets you add references first and delete the plaintext annotations afterwards. The metric `hugin_widget_credentials{source="annotation"}` counts widgets still using a plaintext credential; when it is zero the plaintext annotations can go.

Secrets are cached for five minutes, so a rotated key is picked up within that time.

## Widgets

| `widget.type` | Shows | Parameters besides `url` |
| --- | --- | --- |
| `argocd` | Apps, Synced, Out Of Sync, Healthy | `key` (API token). `fields` may select from `apps`, `synced`, `outOfSync`, `healthy`, `degraded`, `progressing`, `missing`, `suspended`. |
| `grafana` | Dashboards, Data Sources, Total Alerts, Alerts Triggered | `username`, `password` (an admin, for `/api/admin/stats`). `version: 2` with `alerts: alertmanager` reads an external Alertmanager. |
| `nextcloud` | Free Space, Active Users, Files, Shared Items | `key` (serverinfo token), or `username` and `password`. |
| `prometheus` | Targets Up, Targets Down, Total Targets | none |
| `rutorrent` | Active, Upload, Download | `username`, `password` if the UI is protected. |
| `suwayomi` | Downloaded, Non-Downloaded, Read, Unread | `username`, `password` if set. `fields` may select other counts. |
| `traefik` | Routers, Services, Middleware | `username`, `password` if the API is protected. |
| `fritzbox` | Status, Uptime, Max. Down, Max. Up | `fields` may add `down`, `up`, `received`, `sent`, `externalIPAddress`. Needs "Transmit status information over UPnP" enabled on the box. |
| `pihole` | Queries, Blocked, Gravity | `key` (app password) if one is set. `version: 5` for the old `api.php`. |
| `paperlessngx` | Inbox, Total | `key` (API token), or `username` and `password`. |
| `sonarr` | Wanted, Queued, Series | `key`. `enableQueue: "true"` lists the download queue. |
| `radarr` | Wanted, Missing, Queued, Movies | `key`. `enableQueue: "true"` lists the download queue. |
| `jackett` | Configured, Errored | `key`, or `password` if an admin password is set. |
| `seerr`, `jellyseerr`, `overseerr` | Pending, Approved, Completed | `key`. `fields` may add `available`, `processing`, `issues`. |

A widget shows at most six values; the built-in ones show up to four. Numbers are formatted the way gethomepage formats them.

When an upstream call fails, the card shows "API Error" with a short reason such as `HTTP 401` or `timeout`. The reason never includes the URL, because some APIs carry the key in the query string.

A widget that already shows values keeps them, dimmed, through two failed refreshes in a row. The error replaces them on the third.

### Your own widgets

Every widget is a Lua script. The built-in ones are the files in [`widgets/`](widgets); the API they use is described in [docs/widget-api.md](docs/widget-api.md).

To add one, create a `Widget` resource:

```yaml
apiVersion: hugin.dev/v1alpha1
kind: Widget
metadata:
  name: trackhound
  namespace: trackhound
spec:
  script: |
    local orders = http.get("/orders"):check():json()
    return {
      { label = "Orders", value = fmt.number(#orders) },
    }
```

and select it on the Ingress with `gethomepage.dev/widget.type: trackhound` and a `widget.url`.

hugin looks for a `Widget` with that name in the Ingress's own namespace, then in hugin's namespace, then among the built-in scripts. So a `Widget` named `radarr` replaces the built-in Radarr widget, for one namespace or for all.

A changed script is used at the next refresh, which is within seconds while the page is open.

A script runs in a sandbox:

- It can make HTTP requests only to the host in the service's `widget.url`.
- It never sees a credential. `secret("key")` returns a placeholder that hugin swaps in when the request is sent.
- It has no files, no operating system access and no way to load other code.
- Each run is limited to 12 requests, 32 MiB of memory and a fixed instruction budget.

### Generated widgets

Set `gethomepage.dev/widget.type: auto` and hugin picks the `Widget` whose `spec.image` matches the image the service's pods run. Until one exists, the card says so.

The generator writes those widgets. Enable it in the chart:

```yaml
generator:
  enabled: true
  model: gpt-5-mini
  apiKeySecret:
    name: hugin-ai
    key: OPENAI_API_KEY
```

It works with any OpenAI-compatible chat-completions API; set `generator.apiUrl` for another provider or a local model.

For each `auto` service without a widget, the generator:

1. Finds the image the service's pods run.
2. Probes a few well-known paths, such as `/openapi.json`, without credentials.
3. Asks the model for a script, giving it the script API, three built-in scripts as examples, and the probe results.
4. Runs the script against the service, in the same sandbox the dashboard uses.
5. Sends errors and the request trace back to the model, up to `generator.rounds` times.
6. Stores the first script that returns a sensible card as a `Widget` named `auto-<image>` in hugin's namespace.

`kubectl get widgets -A` lists them with their status. `kubectl get widget <name> -o yaml` shows the script.

Things to know:

- **The model never sees credentials.** They are placeholders in the script and are redacted from response bodies. It does see the rest of what the service answers.
- **A service that needs a credential still needs the annotation.** Add `hugin.dev/widget.key-from-secret` (or `password`, `username`) as for any widget.
- **`gethomepage.dev/widget.hint`** tells the model what you would like to see, in plain words.
- **One widget per image.** Two services running the same image share it.
- **Hand-written widgets win.** The generator never writes over a `Widget` it did not create, and skips an image that already has one with `spec.image` set.
- **Regenerate** by deleting the `Widget`, or by adding the annotation `hugin.dev/regenerate` to it. A change of the image's major version also regenerates.
- **A failed generation** leaves a `Widget` with `status.phase: Failed` and the reason, and is retried after `generator.retryHours`.
- **The dashboard does not need the generator.** It runs as a separate Deployment and only writes `Widget` resources.

To try a generation without storing anything:

```bash
hugin draft <namespace>/<ingress> --ai-model gpt-5-mini --hint "queue length and failures"
```

This prints the script, which you can paste into a `Widget` and edit. It needs `HUGIN_AI_KEY` and a kubeconfig.

## Configuration

The chart renders its `config` value to `/config/config.json`. All keys are optional.

| Key | Meaning | Default |
| --- | --- | --- |
| `title` | Page title. | `Hugin` |
| `columns` | Most group columns on a wide screen. | `5` |
| `showStats` | Show pod CPU and memory on each discovered service. | `true` |
| `groupOrder` | Groups to show first, in this order. The rest follow alphabetically. | `[]` |
| `discovery.ingress` | Read Ingress annotations. | `true` |
| `discovery.traefikIngressRoutes` | Also read annotations on Traefik `IngressRoute` objects. | `false` |
| `discovery.widgets` | Load widget scripts from `Widget` resources. | `true` |
| `header.cluster.show`, `header.cluster.label` | Cluster-wide CPU and memory. | `true`, `cluster` |
| `header.nodes.show` | CPU and memory per node. | `true` |
| `header.weather` | `label`, `latitude`, `longitude`, `timezone`, `units` (`metric` or `imperial`), `cacheMinutes`, `maximumFractionDigits`. | off |
| `header.search` | `provider` (`google`, `duckduckgo`, `bing`, `brave`, or `custom` with `url`), `target`. | off |
| `bookmarks` | List of `{ group, items: [{ name, href, abbr, icon }] }`. | `[]` |
| `services` | Static entries, see below. | `[]` |
| `intervals.widgetsSeconds` | How often widgets refresh. | `30` |
| `intervals.resourcesSeconds` | How often node and pod usage refresh. | `15` |
| `intervals.idleAfterSeconds` | Polling stops this long after the last page view. | `90` |
| `intervals.requestTimeoutSeconds` | Timeout for one upstream request. | `10` |

Unknown keys are rejected at startup, so a typo does not pass silently.

### Static services

Entries for things that are not behind an Ingress:

```yaml
config:
  services:
    - group: Infra
      name: Pihole
      href: http://192.168.178.2/
      icon: pi-hole.svg
      description: DNS Adblocker
      widget:
        type: pihole
        params:
          url: http://192.168.178.2
          version: 6
        secrets:
          key: { namespace: dns, name: pihole, key: password }
```

Static entries have no status dot. They sort after discovered services, in the order written, unless you set `weight`.

## Endpoints

| Port | Path | Purpose |
| --- | --- | --- |
| 8080 | `/` | The dashboard. |
| 8080 | `/events` | Server-sent events with re-rendered fragments. |
| 8080 | `/api/state` | The dashboard as JSON: services, status, widget values. No credentials. |
| 8080, 8081 | `/healthz` | Liveness. |
| 8081 | `/metrics` | Prometheus metrics. Set `podMonitor.enabled` for a PodMonitor. |

Port 8081 is not behind the Ingress.

### Metrics

| Metric | Meaning |
| --- | --- |
| `hugin_widget_fetches_total{widget,outcome}` | Widget refreshes by `ok` or `error`. |
| `hugin_widget_fetch_seconds{widget}` | Duration of one widget refresh. |
| `hugin_widget_credentials{source}` | Widgets by credential source: `secret`, `annotation`, `none`. |
| `hugin_services` | Services on the dashboard. |
| `hugin_sse_clients` | Open live-update connections. |
| `hugin_polling_active` | 1 while upstreams are polled, 0 when nobody is watching. |
| `hugin_page_views_total` | Page renders. |
| `hugin_watch_errors_total{resource}` | Kubernetes watch errors. |
| `hugin_poll_errors_total{resource}` | Failed refreshes of metrics or weather. |
| `process_resident_memory_bytes` | Resident memory. |

## Permissions

The chart creates a ClusterRole with read access to:

- Ingresses, to discover services.
- IngressRoutes, only when `discovery.traefikIngressRoutes` is on.
- `Widget` resources, only when `discovery.widgets` is on.
- Pods, for status dots and CPU limits.
- Nodes, for capacity and readiness.
- `metrics.k8s.io` nodes and pods, for usage.

Secrets are not in the ClusterRole. They are granted by name through `secretAccess`.

The generator, when enabled, has its own service account. It can read Ingresses, Pods and `Widget` resources, read the same named Secrets, and write `Widget` resources in hugin's namespace only.

Anyone who can annotate an Ingress can make Hugin call the URL in `widget.url`. Treat that the same as you treat who may create Ingresses.

Anyone who can create a `Widget` in a namespace can run a script against the services of that namespace, with their widget credentials attached to its requests. A `Widget` in hugin's own namespace applies to every namespace.

## Differences from gethomepage

- Memory figures use 1024 for `Ki`, `Mi` and `Gi`. gethomepage multiplies by 1000, so its memory numbers read about 2 to 7 percent lower.
- The Argo CD widget asks for only the sync and health fields, which turns a multi-megabyte response into a few kilobytes.
- The ruTorrent widget sends a form content type with its request.
- Values refresh on the server on a timer and are shared by all viewers, instead of each browser polling.
- Each request to a service opens a new connection. Idle connections are not kept.
- Widgets are Lua scripts, so new ones do not need a release. Only the fourteen listed above are built in.

## Development

```bash
cargo test
cargo run -- --config ./config.json --bind 127.0.0.1:8080 --ops-bind 127.0.0.1:8081
```

With a kubeconfig, Hugin runs against that cluster. `--no-kube` runs without a cluster and shows only static services.

Each built-in widget is a Lua script in `widgets/`, compiled into the binary and tested in `src/widgets/bundled_tests.rs` against a recorded upstream response from `fixtures/`. Scripts reach the network only through the `Http` trait, which the tests replace.

To add a built-in widget: write `widgets/<name>.lua`, add its name to `BUNDLED` in `src/widgets/mod.rs`, and add a fixture and a test.
