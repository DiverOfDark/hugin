# Hugin

Hugin is a small dashboard for a Kubernetes homelab. It lists your services by reading annotations on Ingresses, shows whether their pods are up, and adds a few numbers from each service's own API.

It is one Rust binary that renders HTML on the server and pushes updates to the browser over server-sent events. On a cluster with about 400 pods and 38 services it uses about 10 MiB of memory and 10 millicores while the page is open.

Hugin reads the `gethomepage.dev/*` annotations of [gethomepage](https://gethomepage.dev), so a cluster annotated for gethomepage works without changing any manifest.

## Features

- Service discovery from Ingress annotations, and optionally from Traefik `IngressRoute` objects.
- A status dot per service from pod phase, plus CPU and memory of the pods.
- Widgets are sandboxed Lua scripts stored in the cluster as `Widget` resources. None are built in.
- A generator writes the widget for each service with a language model and validates it against the live service. For an application gethomepage has a widget for, it ports that one.
- Your own widgets, written by hand, take precedence.
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

A widget is a Lua script that fetches a few numbers from a service's API for its card. Scripts live in the cluster as `Widget` resources (`widgets.hugin.dev`). hugin has none built in: the generator writes them, or you do.

A service gets a widget when its Ingress has a `widget.type`:

```yaml
gethomepage.dev/widget.type: radarr
gethomepage.dev/widget.url: http://radarr.media.svc.cluster.local
hugin.dev/widget.key-from-secret: plex-secret/RADARR_API_KEY
```

| Annotation | Meaning |
| --- | --- |
| `widget.type` | What the service is. A gethomepage widget type such as `radarr`, any name of your own, or `auto`. |
| `widget.url` | Base URL of the service's API. Scripts can call this host only. |
| `widget.hint` | Plain words for the generator: what you want to see, or how the API is reached. |
| `widget.<field>` | Other parameters, passed to the script as `widget.params.<field>`. |
| `hugin.dev/widget.<field>-from-secret` | A credential, see [Credentials from Secrets](#credentials-from-secrets). |

For a service, hugin uses the first of:

1. A `Widget` named like the `widget.type`, in the service's namespace.
2. A `Widget` with that name in hugin's namespace.
3. The `Widget` the generator wrote for the service.

Until one exists the card says it is waiting, or why generation failed.

A widget shows at most six values. When an upstream call fails, the card shows "API Error" with a short reason such as `HTTP 401` or `timeout`. The reason never includes the URL, because some APIs carry the key in the query string.

A widget that already shows values keeps them, dimmed, through two failed refreshes in a row. The error replaces them on the third.

### The generator

Enable it in the chart:

```yaml
generator:
  enabled: true
  model: gpt-5-mini
  apiKeySecret:
    name: hugin-ai
    key: OPENAI_API_KEY
```

It works with any OpenAI-compatible chat-completions API; set `generator.apiUrl` for another provider or a local model.

For each service with a `widget.type`, the generator:

1. Probes a few well-known paths of the service, such as `/openapi.json`, without credentials.
2. Fetches gethomepage's source for that widget type from GitHub, if there is one, as the reference to port.
3. Asks the model for a script, giving it the [script API](docs/widget-api.md), the probe results, the reference and your hint.
4. Runs the script against the service, in the same sandbox the dashboard uses.
5. Sends errors, the request trace and response sizes back to the model, up to `generator.rounds` times.
6. Stores the first script that returns a sensible, small enough card.

The `Widget` is named `auto-<ingress name>` and lives in the service's namespace. It has an owner reference to the Ingress, so Argo CD lists it under the Ingress and it is deleted with it. Widgets for static services are owned by hugin's ConfigMap.

`kubectl get widgets -A` lists them with their status. `kubectl get widget <name> -o yaml` shows the script and what it returned when it was validated.

Things to know:

- **The model never sees credentials.** They are placeholders in the script and are redacted from response bodies. It does see the rest of what the service answers.
- **"Validated" means the script ran and returned values.** It does not prove a value means what its label says. Read a generated script once.
- **Known applications work best.** With a gethomepage reference the model ports a working widget. For anything else it relies on the probes, an OpenAPI document if the service has one, and your hint.
- **Use `widget.hint` when it gets something wrong**, for example to name the endpoint or say that no API key is set.
- **A script is written again** when the service's widget annotations change, on a major version bump of its image, when the `Widget` is deleted or annotated `hugin.dev/regenerate`, or when the stored script fails two re-checks in a row. Re-checks run every `generator.revalidateHours`.
- **A failed generation** leaves a `Widget` with `status.phase: Failed` and the reason, and is retried after `generator.retryHours`. A script that worked before is kept.
- **A script that downloads a lot** is sent back for a lighter one twice, then kept if the model finds none.
- **Generated widgets no service asks for are deleted.**
- **The dashboard does not need the generator.** It runs as a separate Deployment and only writes `Widget` resources.
- `generator.referenceUrl: ""` turns the gethomepage reference off.

To try a generation without storing anything:

```bash
hugin draft <namespace>/<ingress> --ai-model gpt-5-mini --hint "queue length and failures"
```

This prints the script. It needs `HUGIN_AI_KEY` and a kubeconfig; `--url` points it at a port-forward.

### Writing a widget by hand

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

Select it on the Ingress with `gethomepage.dev/widget.type: trackhound`. The generator leaves a service alone once a hand-written `Widget` matches its type.

A `Widget` in hugin's namespace applies to every service with that `widget.type`, in any namespace.

The script API is described in [docs/widget-api.md](docs/widget-api.md). A changed script is used at the next refresh, which is within seconds while the page is open.

### The sandbox

- A script can make HTTP requests only to the host in the service's `widget.url`.
- It never sees a credential. `secret("key")` returns a placeholder that hugin swaps in when the request is sent.
- It has no files, no operating system access and no way to load other code.
- Each run is limited to 12 requests, 32 MiB of memory and a fixed instruction budget.

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

The generator, when enabled, has its own service account. It can read Ingresses, Pods and the same named Secrets, read hugin's ConfigMap, and write and delete `Widget` resources in any namespace, because a widget lives next to its Ingress.

Anyone who can annotate an Ingress can make Hugin call the URL in `widget.url`. Treat that the same as you treat who may create Ingresses.

Anyone who can create a `Widget` in a namespace can run a script against the services of that namespace, with their widget credentials attached to its requests. A `Widget` in hugin's own namespace applies to every namespace.

## Differences from gethomepage

- Memory figures use 1024 for `Ki`, `Mi` and `Gi`. gethomepage multiplies by 1000, so its memory numbers read about 2 to 7 percent lower.
- Values refresh on the server on a timer and are shared by all viewers, instead of each browser polling.
- Each request to a service opens a new connection. Idle connections are not kept.
- Widgets are generated Lua scripts, not code in the dashboard. What a card shows for a given application can differ from gethomepage's widget.

## Development

```bash
cargo test
cargo run -- --config ./config.json --bind 127.0.0.1:8080 --ops-bind 127.0.0.1:8081
```

With a kubeconfig, Hugin runs against that cluster. `--no-kube` runs without a cluster and shows only static services.

The Lua sandbox is in `src/widgets/lua.rs`, the generator in `src/generator.rs`. Both reach the network only through the `Http` trait, which the tests replace: the sandbox tests cover the limits and the handling of credentials, and the generator tests drive the whole loop with a scripted model.
