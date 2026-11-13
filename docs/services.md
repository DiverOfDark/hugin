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
| `description` | Second line under the name. | none |
| `pod-selector` | Label selector for the pods behind the service. | `app.kubernetes.io/name=<app>` |
| `app` | Name used in the default pod selector. | the Ingress name |
| `weight` | Sort order inside the group, then by name. | `0` |
| `widget.type` | Widget to show, see [Widgets](widgets.md). | none |
| `widget.url` | Base URL of the service's API. | none |
| `widget.<field>` | Any other widget parameter, such as `key`, `username`, `password`, `fields`, `enableQueue`. | none |

Pods are looked up in the Ingress's own namespace.

A service is running when every matching pod is in phase `Running` or `Succeeded`. It needs attention when only some are, when none are, or when no pod matches. Pods in phase `Failed` are ignored while other pods match: they are leftovers (of a node shutdown, for instance) that their controller has already replaced.

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

Static entries have no status dot. They sort after discovered services, in the order written, unless you set `weight`.
