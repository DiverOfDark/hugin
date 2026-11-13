# Permissions

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
