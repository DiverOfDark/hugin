# Services

Hugin lists a service when its Ingress carries annotations with the `hugin.dev/` prefix.

## Annotations

Put these on an Ingress, each with the `hugin.dev/` prefix.

| Annotation | Meaning | Default |
| --- | --- | --- |
| `enabled` | Must be `"true"` for the service to be listed. | not listed |
| `name` | Display name. | the Ingress name |
| `group` | Column the service appears in. | `Kubernetes` |
| `href` | Link target. | the first Ingress host |
| `icon` | Icon reference, see [Icons](icons.md). | none |
| `description` | Second line under the name. | what the generated widget says the service is, see [The generator](generator.md) |
| `pod-selector` | Label selector for the pods behind the service. | `app.kubernetes.io/name=<app>` |
| `app` | Name used in the default pod selector. | the Ingress name |
| `weight` | Sort order inside the group, then by name. | `0` |
| `probe` | `"false"` to not ask the page at `href` whether it answers. | asked |
| `widget.type` | Widget to show, see [Widgets](widgets.md). | none |
| `widget.url` | Base URL of the service's API. | none |
| `widget.<field>` | Any other widget parameter, such as `key`, `username`, `password`, `fields`, `enableQueue`. | none |

Pods are looked up in the Ingress's own namespace.

A service is running when every matching pod is in phase `Running` or `Succeeded`. It needs attention when only some are, when none are, or when no pod matches. Pods in phase `Failed` are ignored while other pods match: they are leftovers (of a node shutdown, for instance) that their controller has already replaced.

## The light on a card

The light is green when the pods are up and the page answers. Otherwise the card says what is wrong:

| Card says | Meaning | Colour |
| --- | --- | --- |
| `partial` | Only some of the pods are up. | amber |
| `not found` | No pod matches the selector. | amber |
| `down` | No pod is up. | red |
| `HTTP 502`, `timeout`, `connection failed` | The pods are up, but the page at `href` does not answer. | red |

Hugin sends one GET to every card's `href` each minute while someone has the dashboard open. Any status below 500 counts as an answer, a login page or a redirect to a single sign-on included: the question is whether something is serving, not whether Hugin may come in. A page has to fail twice in a row before its card says so.

The requests come from the Hugin pod, so `href` must be reachable from inside the cluster. Set `hugin.dev/probe: "false"` on a service where it is not, or `probes: false` in the [configuration](configuration.md) to turn this off everywhere.

A widget value can also report a problem, see [Widgets](widgets.md#values-that-report-a-problem). The card then gets an amber or red edge while its light stays green: the service runs, and it has something to tell you.

The line at the top of the page counts every card that is not plain green, and the page takes its tint from the worst of them.

## Static services

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

A static entry has no pods, so its light only says whether the page at `href` answers. Set `probe: false` on an entry to leave it without a light. Static entries sort after discovered services, in the order written, unless you set `weight`.
