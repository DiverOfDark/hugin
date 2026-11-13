# Endpoints

| Port | Path | Purpose |
| --- | --- | --- |
| 8080 | `/` | The dashboard. |
| 8080 | `/events` | Server-sent events with re-rendered fragments. |
| 8080 | `/icons/<id>` | A service's cached favicon. |
| 8080 | `/api/state` | The dashboard as JSON: services, status, whether each page answers, widget values and their severity. No credentials. |
| 8080, 8081 | `/healthz` | Liveness. |
| 8081 | `/metrics` | Prometheus metrics. Set `podMonitor.enabled` for a PodMonitor. |

Port 8081 is not behind the Ingress.

## Metrics

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
